//! In-world previews for abilities placed with more than one press: the
//! barrier or ceiling you're laying out (from its first corner to where
//! you aim) and the rift gate you've set (with a ghost of where its link would go).
//! Drawn as thin glowing frames so they show the result without hiding
//! the view.

use glam::{Mat4, Quat, Vec3, Vec4};

use crate::mesh::{BoxUv, MeshData};
use crate::model::MaterialData;
use crate::renderer::Renderer;

/// Glowing bars for the barrier frame and for the ceiling's outline.
const BARS: usize = 12;
const SLAB_BARS: usize = 14;
/// Dots along the line between the two gates.
const DOTS: usize = 16;
const RING_RADIUS: f32 = 1.25;

/// A barrier being laid out: the first corner, where the second would go
/// (as aimed), and the wall's height.
pub struct BarrierPlan {
    pub first: Vec3,
    pub second: Vec3,
    pub height: f32,
    /// Too short to build here.
    pub invalid: bool,
}

/// A ceiling being laid out: two opposite corners of its rectangle (at the
/// slab's height; the second as aimed).
pub struct CeilingPlan {
    pub first: Vec3,
    pub second: Vec3,
    /// Too small to build.
    pub invalid: bool,
}

/// A rift gate waiting for its link: the first gate (centre and facing)
/// and where the second would go, if you're aiming at something.
pub struct GatePlan {
    pub first: Vec3,
    pub first_yaw: f32,
    pub second: Option<(Vec3, f32)>,
    /// Too close to the first to link.
    pub invalid: bool,
}

/// Runes lying in the world: a glowing circle each, in its element's
/// colour, spinning and pulsing faster as its fuse runs out.
pub struct RuneMarks {
    group: usize,
    time: f32,
}

impl RuneMarks {
    pub fn new(renderer: &mut Renderer) -> Self {
        // A flat ring 1 m across with a smaller ring inside and four spokes.
        let mut ring = MeshData::default();
        for (radius, thick) in [(1.0, 0.07), (0.62, 0.045)] {
            let segments = 32;
            for k in 0..segments {
                let a = |i: u32| i as f32 / segments as f32 * std::f32::consts::TAU;
                let p0 = Vec3::new(a(k).cos(), 0.0, a(k).sin()) * radius;
                let p1 = Vec3::new(a(k + 1).cos(), 0.0, a(k + 1).sin()) * radius;
                ring.push_cylinder(p0, p1 - p0, thick, p0.distance(p1), 5, Vec4::ONE, 1.0);
            }
        }
        for k in 0..4 {
            let a = k as f32 * std::f32::consts::FRAC_PI_2;
            let d = Vec3::new(a.cos(), 0.0, a.sin());
            ring.push_cylinder(d * 0.62, d, 0.035, 0.38, 5, Vec4::ONE, 1.0);
        }
        let material = MaterialData { base_color: Vec4::ONE, ground_occlusion: 0.0, ..Default::default() };
        let group = renderer.add_dynamic_group(&ring, &material, 48);
        Self { group, time: 0.0 }
    }

    /// Each circle: where, its colour, fuse left (1 full .. 0), and size (m).
    pub fn update(&mut self, renderer: &mut Renderer, dt: f32, runes: &[(Vec3, Vec4, f32, f32)]) {
        self.time += dt;
        let marks: Vec<crate::renderer::Instance> = runes
            .iter()
            .enumerate()
            .map(|(i, &(at, color, left, size))| {
                let urgency = 1.0 - left.clamp(0.0, 1.0);
                let spin = Quat::from_rotation_y(self.time * (0.8 + urgency * 4.0) + i as f32);
                let pulse = 1.0 + 0.08 * (self.time * (3.0 + urgency * 12.0)).sin();
                let glow = 0.7 + 0.6 * urgency + 0.3 * (self.time * 6.0).sin().abs();
                crate::renderer::Instance {
                    transform: Mat4::from_scale_rotation_translation(Vec3::splat(size * pulse), spin, at + Vec3::Y * 0.12),
                    tint: Vec4::new(color.x * glow, color.y * glow, color.z * glow, 1.0),
                }
            })
            .collect();
        renderer.set_instances(self.group, &marks);
    }
}

pub struct Preview {
    bars: Vec<usize>,
    slab_bars: Vec<usize>,
    rings: [usize; 2],
    dots: Vec<usize>,
    time: f32,
}

fn glow(x: f32) -> Vec4 {
    Vec4::new(x, x, x, 1.0)
}

/// Red for a spot that won't work.
fn bad(x: f32) -> Vec4 {
    Vec4::new(x * 1.6, x * 0.25, x * 0.2, 1.0)
}

impl Preview {
    pub fn new(renderer: &mut Renderer) -> Self {
        let material = MaterialData { base_color: Vec4::new(0.45, 1.7, 2.8, 1.0), ground_occlusion: 0.0, ..Default::default() };
        let mut unit = MeshData::default();
        unit.push_box(Vec3::ZERO, Vec3::ONE, Vec4::ONE, BoxUv::PerFace);
        // A ring standing upright, facing +X, built from short segments.
        let mut ring = MeshData::default();
        let segments = 28;
        for k in 0..segments {
            let a = |i: u32| i as f32 / segments as f32 * std::f32::consts::TAU;
            let p0 = Vec3::new(0.0, a(k).sin(), a(k).cos()) * RING_RADIUS;
            let p1 = Vec3::new(0.0, a(k + 1).sin(), a(k + 1).cos()) * RING_RADIUS;
            ring.push_cylinder(p0, p1 - p0, 0.06, p0.distance(p1), 6, Vec4::ONE, 1.0);
        }
        let mut hidden = |mesh: &MeshData| {
            let h = renderer.add_dynamic_mesh(mesh, &material);
            renderer.set_transform(h, Mat4::IDENTITY, false);
            h
        };
        let bars = (0..BARS).map(|_| hidden(&unit)).collect();
        let slab_bars = (0..SLAB_BARS).map(|_| hidden(&unit)).collect();
        let rings = [hidden(&ring), hidden(&ring)];
        let dots = (0..DOTS).map(|_| hidden(&unit)).collect();
        Self { bars, slab_bars, rings, dots, time: 0.0 }
    }

    /// Shows (or hides) the previews for this frame.
    pub fn update(&mut self, renderer: &mut Renderer, dt: f32, barrier: Option<BarrierPlan>, ceiling: Option<CeilingPlan>, gate: Option<GatePlan>) {
        self.time += dt;
        let pulse = 0.75 + 0.35 * (self.time * 5.0).sin();
        let mut used = 0;
        if let Some(b) = barrier {
            let color = if b.invalid { bad(pulse) } else { glow(pulse) };
            let span = Vec3::new(b.second.x - b.first.x, 0.0, b.second.z - b.first.z);
            let width = span.length().max(0.01);
            let along = span / width;
            let yaw = Quat::from_rotation_arc(Vec3::Z, along);
            let mut bar = |renderer: &mut Renderer, center: Vec3, size: Vec3, rot: Quat| {
                if let Some(&h) = self.bars.get(used) {
                    renderer.set_transform_tinted(h, Mat4::from_scale_rotation_translation(size, rot, center), color, true);
                    used += 1;
                }
            };
            // Corner posts, the top and bottom rails, and a few slats.
            let (a, z) = (b.first, b.first + span);
            // Like the wall itself: standing from the lower corner.
            let ground = b.first.y.min(b.second.y);
            for p in [a, z] {
                bar(renderer, Vec3::new(p.x, ground + b.height * 0.5, p.z), Vec3::new(0.14, b.height, 0.14), Quat::IDENTITY);
            }
            let mid = (a + z) * 0.5;
            for h in [0.08, b.height] {
                bar(renderer, Vec3::new(mid.x, ground + h, mid.z), Vec3::new(0.08, 0.08, width), yaw);
            }
            let slats = ((width / 3.0).floor() as usize).min(BARS - 4);
            for k in 1..=slats {
                let p = a + span * (k as f32 / (slats + 1) as f32);
                bar(renderer, Vec3::new(p.x, ground + b.height * 0.5, p.z), Vec3::new(0.04, b.height, 0.04), Quat::IDENTITY);
            }
        }
        for &h in &self.bars[used..] {
            renderer.set_transform(h, Mat4::IDENTITY, false);
        }

        // The ceiling: its outline, a lattice across it, and short drops
        // at the corners so you can judge its height.
        let mut used = 0;
        if let Some(c) = ceiling {
            let color = if c.invalid { bad(pulse) } else { glow(pulse) };
            let y = c.first.y.max(c.second.y);
            let (lo, hi) = (c.first.min(c.second), c.first.max(c.second));
            let (w, d) = ((hi.x - lo.x).max(0.05), (hi.z - lo.z).max(0.05));
            let mid = Vec3::new((lo.x + hi.x) * 0.5, y, (lo.z + hi.z) * 0.5);
            let mut bar = |renderer: &mut Renderer, center: Vec3, size: Vec3| {
                if let Some(&h) = self.slab_bars.get(used) {
                    renderer.set_transform_tinted(h, Mat4::from_scale_rotation_translation(size, Quat::IDENTITY, center), color, true);
                    used += 1;
                }
            };
            for z in [lo.z, hi.z] {
                bar(renderer, Vec3::new(mid.x, y, z), Vec3::new(w, 0.1, 0.1));
            }
            for x in [lo.x, hi.x] {
                bar(renderer, Vec3::new(x, y, mid.z), Vec3::new(0.1, 0.1, d));
            }
            for k in 1..=3 {
                let f = k as f32 / 4.0;
                bar(renderer, Vec3::new(mid.x, y, lo.z + d * f), Vec3::new(w, 0.04, 0.04));
                bar(renderer, Vec3::new(lo.x + w * f, y, mid.z), Vec3::new(0.04, 0.04, d));
            }
            for (x, z) in [(lo.x, lo.z), (lo.x, hi.z), (hi.x, lo.z), (hi.x, hi.z)] {
                bar(renderer, Vec3::new(x, y - 0.6, z), Vec3::new(0.05, 1.2, 0.05));
            }
        }
        for &h in &self.slab_bars[used..] {
            renderer.set_transform(h, Mat4::IDENTITY, false);
        }

        let mut dots_used = 0;
        let mut rings_used = 0;
        if let Some(g) = gate {
            // The first gate spins slowly to show it's waiting.
            let spin = Quat::from_rotation_x(self.time * 1.5);
            let set = Mat4::from_scale_rotation_translation(Vec3::ONE, Quat::from_rotation_y(-g.first_yaw) * spin, g.first);
            renderer.set_transform_tinted(self.rings[0], set, glow(pulse * 1.3), true);
            rings_used = 1;
            if let Some((pos, yaw)) = g.second {
                let color = if g.invalid { bad(pulse) } else { glow(pulse * 0.6) };
                let ghost = Mat4::from_scale_rotation_translation(Vec3::splat(0.9), Quat::from_rotation_y(-yaw), pos);
                renderer.set_transform_tinted(self.rings[1], ghost, color, true);
                rings_used = 2;
                // Dotted line between them, marching from first to second.
                let to = pos - g.first;
                for (k, &h) in self.dots.iter().enumerate() {
                    let f = ((k as f32 + (self.time * 2.0).fract()) / DOTS as f32).min(1.0);
                    let t = Mat4::from_scale_rotation_translation(Vec3::splat(0.12), Quat::IDENTITY, g.first + to * f);
                    renderer.set_transform_tinted(h, t, color, true);
                }
                dots_used = DOTS;
            }
        }
        for &h in &self.rings[rings_used..] {
            renderer.set_transform(h, Mat4::IDENTITY, false);
        }
        for &h in &self.dots[dots_used..] {
            renderer.set_transform(h, Mat4::IDENTITY, false);
        }
    }
}
