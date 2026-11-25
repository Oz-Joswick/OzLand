//! Gadgets: bounce pads, the grapple hook (its aiming ray and rope), and
//! the Time tree's recall marker. All drawn as small glowing dynamic meshes.

use glam::{Mat4, Quat, Vec3, Vec4};

use crate::collision::Aabb;
use crate::mesh::{BoxUv, MeshData};
use crate::model::MaterialData;
use crate::player::Player;
use crate::renderer::Renderer;
use crate::terrain::Terrain;

/// Most pads that can be out at once (Super Pads level).
pub const MAX_PADS: usize = 3;
const PAD_RADIUS: f32 = 0.8;

fn glowing(color: Vec4) -> MaterialData {
    MaterialData {
        base_color: color,
        specular: 0.6,
        shininess: 40.0,
        ground_occlusion: 0.0,
        ..Default::default()
    }
}

/// The first thing a ray from `origin` along `dir` hits within `max`
/// meters: a collider or the terrain.
pub fn raycast(origin: Vec3, dir: Vec3, max: f32, colliders: &[Aabb], terrain: &Terrain) -> Option<Vec3> {
    let dir = dir.normalize_or_zero();
    if dir == Vec3::ZERO {
        return None;
    }
    let mut best = max;
    for c in colliders {
        if let Some(t) = ray_box(origin, dir, c)
            && t < best
        {
            best = t;
        }
    }
    let mut hit = (best < max).then_some(best);

    // March over the terrain (it's under any collider hit we already have).
    let below = |t: f32| {
        let p = origin + dir * t;
        p.y < terrain.height_at(p.x, p.z)
    };
    let mut t = 0.0;
    while t < best {
        let next = (t + 0.5).min(best);
        if below(next) {
            // Narrow down where the ray crosses the ground.
            let (mut lo, mut hi) = (t, next);
            for _ in 0..12 {
                let mid = (lo + hi) * 0.5;
                if below(mid) {
                    hi = mid;
                } else {
                    lo = mid;
                }
            }
            hit = Some(hi);
            break;
        }
        t = next;
    }
    hit.map(|t| origin + dir * t)
}

/// Distance along the ray to the box, if it's hit (from outside).
fn ray_box(origin: Vec3, dir: Vec3, b: &Aabb) -> Option<f32> {
    let (mut near, mut far) = (0.0f32, f32::INFINITY);
    for axis in 0..3 {
        let (o, d, lo, hi) = (origin[axis], dir[axis], b.min[axis], b.max[axis]);
        if d.abs() < 1e-8 {
            if o < lo || o > hi {
                return None;
            }
            continue;
        }
        let (mut t0, mut t1) = ((lo - o) / d, (hi - o) / d);
        if t0 > t1 {
            std::mem::swap(&mut t0, &mut t1);
        }
        near = near.max(t0);
        far = far.min(t1);
        if near > far {
            return None;
        }
    }
    (near > 0.0).then_some(near)
}

pub struct Gadgets {
    pads: Vec<Vec3>,
    pad_handles: Vec<usize>,
    rope: usize,
    hook_point: usize,
    marker: usize,
    time: f32,
}

impl Gadgets {
    pub fn new(renderer: &mut Renderer) -> Self {
        let mut pad = MeshData::default();
        pad.push_cylinder(Vec3::ZERO, Vec3::Y, PAD_RADIUS, 0.12, 24, Vec4::ONE, 1.0);
        let pad_material = glowing(Vec4::new(2.6, 1.0, 0.25, 1.0));
        let pad_handles = (0..MAX_PADS).map(|_| renderer.add_dynamic_mesh(&pad, &pad_material)).collect();

        // A thin beam from z = 0 to z = 1, stretched to length each frame.
        let mut rope = MeshData::default();
        rope.push_box(Vec3::new(0.0, 0.0, 0.5), Vec3::new(0.05, 0.05, 1.0), Vec4::ONE, BoxUv::PerFace);
        let rope = renderer.add_dynamic_mesh(&rope, &glowing(Vec4::new(1.2, 1.0, 0.8, 1.0)));
        let mut point = MeshData::default();
        point.push_box(Vec3::ZERO, Vec3::splat(0.3), Vec4::ONE, BoxUv::PerFace);
        let hook_point = renderer.add_dynamic_mesh(&point, &glowing(Vec4::new(2.0, 1.6, 0.6, 1.0)));

        let mut beacon = MeshData::default();
        beacon.push_cylinder(Vec3::ZERO, Vec3::Y, 0.12, 3.0, 12, Vec4::ONE, 1.0);
        beacon.push_cylinder(Vec3::ZERO, Vec3::Y, 0.6, 0.08, 20, Vec4::ONE, 1.0);
        let marker = renderer.add_dynamic_mesh(&beacon, &glowing(Vec4::new(0.4, 2.6, 0.9, 1.0)));

        let mut gadgets = Self {
            pads: Vec::new(),
            pad_handles,
            rope,
            hook_point,
            marker,
            time: 0.0,
        };
        gadgets.hide_all(renderer);
        gadgets
    }

    fn hide_all(&mut self, renderer: &mut Renderer) {
        for &h in self.pad_handles.iter().chain([&self.rope, &self.hook_point, &self.marker]) {
            renderer.set_transform(h, Mat4::IDENTITY, false);
        }
    }

    pub fn pad_count(&self) -> usize {
        self.pads.len()
    }

    /// Drops a pad at `feet`; past `max`, the oldest is picked up again.
    pub fn place_pad(&mut self, feet: Vec3, max: usize) {
        let max = max.min(MAX_PADS);
        if max == 0 {
            return;
        }
        while self.pads.len() >= max {
            self.pads.remove(0);
        }
        self.pads.push(feet);
    }

    pub fn clear_pads(&mut self) {
        self.pads.clear();
    }

    /// Launches `player` if they're standing on a pad (or falling onto one).
    pub fn bounce(&self, player: &mut Player, launch: f32) -> bool {
        let on_pad = self.pads.iter().any(|p| {
            let d = player.pos - *p;
            Vec3::new(d.x, 0.0, d.z).length() < PAD_RADIUS && (-0.3..0.4).contains(&d.y) && player.velocity().y <= 0.5
        });
        if on_pad {
            player.launch(launch);
        }
        on_pad
    }

    /// Draws the pads (pulsing), the grapple rope from `hand` to `hook`,
    /// and the recall marker.
    pub fn draw(&mut self, renderer: &mut Renderer, dt: f32, hand: Vec3, hook: Option<Vec3>, marker: Option<Vec3>) {
        self.time += dt;
        for (i, &h) in self.pad_handles.iter().enumerate() {
            match self.pads.get(i) {
                Some(&p) => {
                    let pulse = 1.0 + (self.time * 5.0 + i as f32).sin() * 0.06;
                    let t = Mat4::from_scale_rotation_translation(Vec3::new(pulse, 1.0, pulse), Quat::IDENTITY, p);
                    renderer.set_transform(h, t, true);
                }
                None => renderer.set_transform(h, Mat4::IDENTITY, false),
            }
        }
        match hook {
            Some(point) => {
                let span = point - hand;
                let rope = Mat4::from_scale_rotation_translation(
                    Vec3::new(1.0, 1.0, span.length()),
                    Quat::from_rotation_arc(Vec3::Z, span.normalize_or(Vec3::Z)),
                    hand,
                );
                renderer.set_transform(self.rope, rope, true);
                renderer.set_transform(self.hook_point, Mat4::from_translation(point), true);
            }
            None => {
                renderer.set_transform(self.rope, Mat4::IDENTITY, false);
                renderer.set_transform(self.hook_point, Mat4::IDENTITY, false);
            }
        }
        match marker {
            Some(p) => {
                let spin = Quat::from_rotation_y(self.time * 2.0);
                renderer.set_transform(self.marker, Mat4::from_rotation_translation(spin, p), true);
            }
            None => renderer.set_transform(self.marker, Mat4::IDENTITY, false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rays_hit_boxes_and_ground() {
        let flat = Terrain::flat(50.0);
        let wall = [Aabb::from_center(Vec3::new(10.0, 2.0, 0.0), Vec3::new(1.0, 4.0, 4.0))];
        let hit = raycast(Vec3::new(0.0, 1.5, 0.0), Vec3::X, 30.0, &wall, &flat).unwrap();
        assert!((hit.x - 9.5).abs() < 1e-3, "{hit}");
        let down = raycast(Vec3::new(0.0, 5.0, 0.0), Vec3::new(1.0, -1.0, 0.0), 30.0, &[], &flat).unwrap();
        assert!(down.y.abs() < 0.01 && (down.x - 5.0).abs() < 0.05, "{down}");
        assert!(raycast(Vec3::new(0.0, 1.5, 0.0), Vec3::NEG_X, 30.0, &wall, &flat).is_none());
    }

    #[test]
    fn pads_launch_players_standing_on_them() {
        let pads = Gadgets {
            pads: vec![Vec3::ZERO],
            pad_handles: Vec::new(),
            rope: 0,
            hook_point: 0,
            marker: 0,
            time: 0.0,
        };
        let mut on = Player::new(Vec3::new(0.3, 0.0, 0.0));
        assert!(pads.bounce(&mut on, 20.0));
        assert!(on.velocity().y > 19.0);
        let mut off = Player::new(Vec3::new(3.0, 0.0, 0.0));
        assert!(!pads.bounce(&mut off, 20.0));
    }
}
