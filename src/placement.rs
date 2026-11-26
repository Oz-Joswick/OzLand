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

/// One spell circle in the world.
#[derive(Clone, Copy)]
pub struct RuneMark {
    /// Its centre.
    pub at: Vec3,
    /// The way it faces: up for one lying on the ground, the spell's aim
    /// for one standing up.
    pub facing: Vec3,
    pub color: Vec4,
    /// Fuse left (1 full .. 0): it spins and pulses faster as it runs out.
    pub left: f32,
    /// Radius (m).
    pub size: f32,
}

/// Spell circles lying in the world, drawn like the casting sigil round
/// the crosshair: two rings with eight angular glyphs between them and a
/// crossed core, in the spell's element colour.
pub struct RuneMarks {
    group: usize,
    time: f32,
}

/// The casting sigil's glyphs (as in the HUD): strokes between points of a
/// small compass (eight around, then the middle).
const GLYPHS: [[usize; 6]; 8] = [
    [0, 8, 4, 8, 2, 0], [6, 8, 1, 8, 5, 6], [0, 3, 6, 1, 4, 0], [2, 7, 4, 1, 6, 2],
    [0, 5, 2, 7, 4, 1], [6, 3, 0, 2, 5, 7], [1, 4, 7, 3, 0, 6], [0, 2, 5, 1, 6, 4],
];
const POINTS: [(f32, f32); 9] = [
    (0.0, -1.0), (0.7, -0.7), (1.0, 0.0), (0.7, 0.7),
    (0.0, 1.0), (-0.7, 0.7), (-1.0, 0.0), (-0.7, -0.7), (0.0, 0.0),
];

/// The sigil, 1 m in radius, lying flat (facing +Y).
fn sigil() -> MeshData {
    let mut mesh = MeshData::default();
    let mut line = |a: Vec3, b: Vec3, thick: f32| {
        if a.distance(b) > 1e-4 {
            mesh.push_cylinder(a, b - a, thick, a.distance(b), 5, Vec4::ONE, 1.0);
        }
    };
    let around = |a: f32| Vec3::new(a.cos(), 0.0, a.sin());
    // Outer and inner rings, and the core's ring.
    for (radius, thick) in [(1.0, 0.026), (0.58, 0.02), (0.29, 0.022)] {
        let segments = 40;
        for k in 0..segments {
            let a = |i: u32| i as f32 / segments as f32 * std::f32::consts::TAU;
            line(around(a(k)) * radius, around(a(k + 1)) * radius, thick);
        }
    }
    // Eight glyphs on a ring between them, each turned to face outwards.
    for (i, glyph) in GLYPHS.iter().enumerate() {
        let a = i as f32 * std::f32::consts::TAU / 8.0;
        let (dir, right) = (around(a), Vec3::new(-a.sin(), 0.0, a.cos()));
        let centre = dir * 0.79;
        let map = |p: usize| centre + (dir * POINTS[p].0 + right * POINTS[p].1) * 0.13;
        for pair in glyph.windows(2) {
            line(map(pair[0]), map(pair[1]), 0.016);
        }
        line(centre - Vec3::Y * 0.02, centre + Vec3::Y * 0.02, 0.03);
    }
    // The crossed core.
    let c = 0.29 * 0.55;
    line(Vec3::new(-c, 0.0, c), Vec3::new(c, 0.0, -c), 0.024);
    line(Vec3::new(-c, 0.0, -c), Vec3::new(c, 0.0, c), 0.018);
    mesh
}

impl RuneMarks {
    pub fn new(renderer: &mut Renderer) -> Self {
        let material = MaterialData { base_color: Vec4::ONE, ground_occlusion: 0.0, ..Default::default() };
        // Yours and four arcane clones', each with a target mark, for
        // every spell you might have planted.
        let group = renderer.add_dynamic_group(&sigil(), &material, 160);
        Self { group, time: 0.0 }
    }

    pub fn update(&mut self, renderer: &mut Renderer, dt: f32, marks: &[RuneMark]) {
        self.time += dt;
        let instances: Vec<crate::renderer::Instance> = marks
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let urgency = 1.0 - m.left.clamp(0.0, 1.0);
                // Turn within its own plane (like the HUD sigil), then face
                // the way it should.
                let spin = Quat::from_rotation_y(self.time * (0.55 + urgency * 4.0) + i as f32);
                let face = Quat::from_rotation_arc(Vec3::Y, m.facing.normalize_or(Vec3::Y));
                let pulse = 1.0 + 0.06 * (self.time * (3.0 + urgency * 12.0)).sin();
                let glow = 0.7 + 0.6 * urgency + 0.3 * (self.time * 6.0).sin().abs();
                crate::renderer::Instance {
                    transform: Mat4::from_scale_rotation_translation(Vec3::splat(m.size * pulse), face * spin, m.at),
                    tint: Vec4::new(m.color.x * glow, m.color.y * glow, m.color.z * glow, 1.0),
                }
            })
            .collect();
        renderer.set_instances(self.group, &instances);
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
