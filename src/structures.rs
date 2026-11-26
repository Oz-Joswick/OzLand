//! Things you build on the map with the Control branch: energy barriers,
//! ceilings that stop whoever is under them from jumping or flying out,
//! and domes nobody can leave (or enter) until they break.
//!
//! Barriers and ceilings are solid boxes added to the world's colliders, so
//! players walk into them, stand on them and bump their heads on them, and
//! the grapple and dashes respect them. Domes are exact hemispheres: each
//! frame [`Structures::constrain`] keeps a player on whichever side of the
//! shell they started the frame. Everything blocks shots and drone fire
//! (see [`Blockers`]) and takes damage until it breaks or times out.

use glam::{Mat4, Quat, Vec3, Vec4};

use crate::collision::Aabb;
use crate::mesh::{BoxUv, MeshData};
use crate::model::MaterialData;
use crate::player::{PLAYER_RADIUS, Player};
use crate::renderer::Renderer;

pub const BARRIER_WIDTH: f32 = 16.0;
pub const BARRIER_HEIGHT: f32 = 10.0;
pub const CEILING_SIZE: f32 = 28.0;
pub const DOME_RADIUS: f32 = 6.0;
/// The Giant Box: walls this wide and tall (and thick), with a floor and roof.
pub const BOX_SIZE: f32 = 30.0;
pub const BOX_HEIGHT: f32 = 20.0;
const BOX_WALL: f32 = 1.0;
/// Shots do this many times their damage to structures.
pub const SHOT_SCALE: f32 = 5.0;
const MAX_BARRIERS: usize = 4;
const MAX_CEILINGS: usize = 2;
const MAX_DOMES: usize = 2;
/// Heal pads and storms: room for a Rejuvenate or Blizzard from you and
/// four arcane clones, twice over (the Heal Pad skill keeps its own two;
/// see `Game::build`).
const MAX_PADS: usize = 12;
const MAX_WELLS: usize = 2;
const MAX_STORMS: usize = 12;
const MAX_BOXES: usize = 2;
pub const STORM_RADIUS: f32 = 12.0;
/// A healing pad's reach and a gravity well's, at size 1.
pub const PAD_RADIUS: f32 = 2.5;
pub const WELL_RADIUS: f32 = 13.0;
/// How long the rise / drop / swell takes when something is built.
const BUILD_TIME: f32 = 0.3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Barrier,
    Ceiling,
    Dome,
    /// Heals whoever stands on it.
    HealPad,
    /// Drags players and drones towards its centre.
    Well,
    /// Blizzard: slows and chills players, freezes drones.
    Storm,
    /// The Giant Box: four massive walls, a floor and a roof.
    Box,
}

impl Kind {
    pub fn code(self) -> u8 {
        match self {
            Kind::Barrier => 0,
            Kind::Ceiling => 1,
            Kind::Dome => 2,
            Kind::HealPad => 3,
            Kind::Well => 4,
            Kind::Storm => 5,
            Kind::Box => 6,
        }
    }

    pub fn from_code(c: u8) -> Option<Kind> {
        [Kind::Barrier, Kind::Ceiling, Kind::Dome, Kind::HealPad, Kind::Well, Kind::Storm, Kind::Box].get(c as usize).copied()
    }
}

/// One built thing. Sent over the network as is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Structure {
    /// Unique across both players (the builder's number in the top byte).
    pub id: u32,
    pub kind: Kind,
    /// Barrier and dome: the middle of the base. Ceiling: the middle of its
    /// underside.
    pub pos: Vec3,
    /// Barrier: which way it faces (the builder's view). Custom ceiling:
    /// its depth along Z (see [`Structure::ceiling_size`]).
    pub yaw: f32,
    pub health: f32,
    pub max_health: f32,
    /// Seconds left before it fades.
    pub life: f32,
    /// Upgraded: Warden domes shrink, squeezing whoever is inside;
    /// Sanctuary pads recharge shields.
    pub warden: bool,
    /// Size (grows with the builder's Ascension).
    pub scale: f32,
    /// Healing per second (pads), pull strength (wells), a custom
    /// barrier's width, or a custom ceiling's width along X.
    pub power: f32,
}

impl Structure {
    /// A ceiling's width (X) and depth (Z): custom ones carry them in
    /// `power` and `yaw`; older ones are the standard square.
    pub fn ceiling_size(&self) -> (f32, f32) {
        if self.power > 0.0 { (self.power, self.yaw) } else { (CEILING_SIZE * self.scale, CEILING_SIZE * self.scale) }
    }

    /// Along the barrier's face.
    fn side(&self) -> Vec3 {
        Vec3::new(-self.yaw.sin(), 0.0, self.yaw.cos())
    }

    /// A dome's radius now (a Warden's shrinks as it ages).
    pub fn radius(&self, age: f32) -> f32 {
        if self.warden {
            let t = (age / (age + self.life).max(0.01)).clamp(0.0, 1.0);
            DOME_RADIUS * (1.2 + (0.5 - 1.2) * t) * self.scale
        } else {
            DOME_RADIUS * self.scale
        }
    }

    /// Where its health bar goes.
    pub fn top(&self) -> Vec3 {
        match self.kind {
            Kind::Barrier => self.pos + Vec3::Y * (BARRIER_HEIGHT * self.scale + 0.4),
            Kind::Ceiling => self.pos + Vec3::Y * 0.8,
            Kind::Dome => self.pos + Vec3::Y * (DOME_RADIUS * self.scale + 0.6),
            Kind::HealPad | Kind::Well | Kind::Storm => self.pos + Vec3::Y * 1.2,
            Kind::Box => self.pos + Vec3::Y * (BOX_HEIGHT * self.scale + 1.5),
        }
    }

    /// Solid boxes (barriers in short overlapping pieces so a wall at any
    /// angle is solid; ceilings as one slab). Domes have none.
    fn boxes(&self) -> Vec<Aabb> {
        match self.kind {
            Kind::Barrier => {
                let side = self.side();
                let w = if self.power > 0.0 { self.power } else { BARRIER_WIDTH * self.scale };
                let (h, thickness) = (BARRIER_HEIGHT * self.scale, 0.55 * self.scale.sqrt());
                let pieces = (w / 0.4).ceil() as usize;
                (0..pieces)
                    .map(|k| {
                        let along = (k as f32 + 0.5) / pieces as f32 * w - w * 0.5;
                        let c = self.pos + side * along + Vec3::Y * h * 0.5;
                        Aabb::from_center(c, Vec3::new(thickness, h, thickness))
                    })
                    .collect()
            }
            Kind::Ceiling => {
                let (w, d) = self.ceiling_size();
                vec![Aabb::from_center(self.pos + Vec3::Y * 0.15, Vec3::new(w, 0.3, d))]
            }
            Kind::Box => {
                let (s, h, t) = (BOX_SIZE * self.scale, BOX_HEIGHT * self.scale, BOX_WALL * self.scale.sqrt());
                let c = self.pos;
                vec![
                    Aabb::from_center(c + Vec3::new(s * 0.5, h * 0.5, 0.0), Vec3::new(t, h, s + t)),
                    Aabb::from_center(c + Vec3::new(-s * 0.5, h * 0.5, 0.0), Vec3::new(t, h, s + t)),
                    Aabb::from_center(c + Vec3::new(0.0, h * 0.5, s * 0.5), Vec3::new(s + t, h, t)),
                    Aabb::from_center(c + Vec3::new(0.0, h * 0.5, -s * 0.5), Vec3::new(s + t, h, t)),
                    Aabb::from_center(c + Vec3::new(0.0, h, 0.0), Vec3::new(s + t, t, s + t)),
                    Aabb::from_center(c - Vec3::Y * (t * 0.5), Vec3::new(s + t, t, s + t)),
                ]
            }
            Kind::Dome | Kind::HealPad | Kind::Well | Kind::Storm => Vec::new(),
        }
    }
}

/// What stops shots: structure boxes and dome shells, by structure id.
#[derive(Clone, Default)]
pub struct Blockers {
    pub boxes: Vec<(Aabb, u32)>,
    /// (centre, radius, id)
    pub domes: Vec<(Vec3, f32, u32)>,
}

impl Blockers {
    /// The first structure a segment from `origin` along unit `dir` meets
    /// within `max`: (distance, id).
    pub fn hit(&self, origin: Vec3, dir: Vec3, max: f32) -> Option<(f32, u32)> {
        let mut best: Option<(f32, u32)> = None;
        let mut consider = |t: f32, id: u32| {
            if (0.0..=max).contains(&t) && best.is_none_or(|b| t < b.0) {
                best = Some((t, id));
            }
        };
        for (b, id) in &self.boxes {
            if let Some(t) = ray_box(origin, dir, b) {
                consider(t, *id);
            }
        }
        for &(c, r, id) in &self.domes {
            // Crossing the shell, either way (only the upper half exists).
            let oc = origin - c;
            let b = oc.dot(dir);
            let disc = b * b - (oc.length_squared() - r * r);
            if disc < 0.0 {
                continue;
            }
            let s = disc.sqrt();
            for t in [-b - s, -b + s] {
                if t >= 0.0 && (origin + dir * t).y >= c.y - 0.2 {
                    consider(t, id);
                    break;
                }
            }
        }
        best
    }
}

/// Slab test: where a ray enters a box (0 if it starts inside).
fn ray_box(origin: Vec3, dir: Vec3, b: &Aabb) -> Option<f32> {
    let inv = dir.recip();
    let t1 = (b.min - origin) * inv;
    let t2 = (b.max - origin) * inv;
    let near = t1.min(t2).max_element();
    let far = t1.max(t2).min_element();
    (far >= near.max(0.0)).then_some(near.max(0.0))
}

struct Built {
    s: Structure,
    age: f32,
    /// Flash after a hit.
    flash: f32,
}

/// Everything built, by both players.
pub struct Structures {
    list: Vec<Built>,
    next: u32,
    /// The colliders changed (rebuild the world's list).
    dirty: bool,
    barrier_handles: Vec<usize>,
    ceiling_handles: Vec<usize>,
    dome_handles: Vec<usize>,
    pad_handles: Vec<usize>,
    well_handles: Vec<usize>,
    storm_handles: Vec<usize>,
    box_handles: Vec<usize>,
    time: f32,
}

fn glow(color: Vec4) -> MaterialData {
    MaterialData {
        base_color: color,
        ground_occlusion: 0.0,
        ..Default::default()
    }
}

/// A lattice wall 4 m wide, 3 m tall, along +X, standing on y = 0.
fn barrier_mesh() -> MeshData {
    let mut m = MeshData::default();
    let (w, h) = (BARRIER_WIDTH, BARRIER_HEIGHT);
    let c = Vec4::ONE;
    for x in [-w * 0.5, w * 0.5] {
        m.push_cylinder(Vec3::new(x, 0.0, 0.0), Vec3::Y, 0.09, h, 8, c, 1.0);
    }
    for i in 0..7 {
        let y = 0.1 + i as f32 * (h - 0.2) / 6.0;
        m.push_cylinder(Vec3::new(-w * 0.5, y, 0.0), Vec3::X, if i == 0 || i == 6 { 0.06 } else { 0.03 }, w, 6, c, 1.0);
    }
    for i in 1..8 {
        let x = -w * 0.5 + i as f32 * w / 8.0;
        m.push_cylinder(Vec3::new(x, 0.0, 0.0), Vec3::Y, 0.02, h, 5, c, 1.0);
    }
    m
}

/// A lattice slab, CEILING_SIZE square, at y = 0.
fn ceiling_mesh() -> MeshData {
    let mut m = MeshData::default();
    let s = CEILING_SIZE;
    let c = Vec4::ONE;
    for i in 0..=8 {
        let t = -s * 0.5 + i as f32 * s / 8.0;
        let r = if i == 0 || i == 8 { 0.08 } else { 0.035 };
        m.push_cylinder(Vec3::new(-s * 0.5, 0.15, t), Vec3::X, r, s, 6, c, 1.0);
        m.push_cylinder(Vec3::new(t, 0.15, -s * 0.5), Vec3::Z, r, s, 6, c, 1.0);
    }
    m
}

/// A lattice hemisphere of radius 1 on y = 0 (scaled to size).
fn dome_mesh() -> MeshData {
    let mut m = MeshData::default();
    let c = Vec4::ONE;
    let point = |az: f32, el: f32| Vec3::new(el.cos() * az.cos(), el.sin(), el.cos() * az.sin());
    let mut bar = |a: Vec3, b: Vec3, r: f32| {
        let d = b - a;
        m.push_cylinder(a, d, r, d.length(), 5, c, 1.0);
    };
    let tau = std::f32::consts::TAU;
    let half_pi = std::f32::consts::FRAC_PI_2;
    // Meridians.
    for k in 0..14 {
        let az = k as f32 / 14.0 * tau;
        for s in 0..7 {
            let (e0, e1) = (s as f32 / 7.0 * half_pi, (s + 1) as f32 / 7.0 * half_pi);
            bar(point(az, e0), point(az, e1), 0.018);
        }
    }
    // Parallels.
    for (j, el) in [0.02f32, 0.35, 0.7, 1.05, 1.3].into_iter().enumerate() {
        for k in 0..28 {
            let (a0, a1) = (k as f32 / 28.0 * tau, (k + 1) as f32 / 28.0 * tau);
            bar(point(a0, el), point(a1, el), if j == 0 { 0.03 } else { 0.015 });
        }
    }
    m
}

/// A healing pad: a glowing ring with a cross, radius PAD_RADIUS, flat.
fn pad_mesh() -> MeshData {
    let mut m = MeshData::default();
    let c = Vec4::ONE;
    let r = PAD_RADIUS;
    let n = 32;
    for k in 0..n {
        let (a0, a1) = (k as f32 / n as f32 * std::f32::consts::TAU, (k + 1) as f32 / n as f32 * std::f32::consts::TAU);
        let (p0, p1) = (Vec3::new(a0.cos() * r, 0.05, a0.sin() * r), Vec3::new(a1.cos() * r, 0.05, a1.sin() * r));
        m.push_cylinder(p0, p1 - p0, 0.07, (p1 - p0).length(), 5, c, 1.0);
    }
    m.push_cylinder(Vec3::new(-0.9, 0.08, 0.0), Vec3::X, 0.18, 1.8, 6, c, 1.0);
    m.push_cylinder(Vec3::new(0.0, 0.08, -0.9), Vec3::Z, 0.18, 1.8, 6, c, 1.0);
    m
}

/// A gravity well: rings and curved spokes swirling in.
fn well_mesh() -> MeshData {
    let mut m = MeshData::default();
    let c = Vec4::ONE;
    let tau = std::f32::consts::TAU;
    for (ring, radius) in [(0, 1.2f32), (1, 3.5), (2, 7.0)] {
        let n = 24 + ring * 12;
        for k in 0..n {
            let (a0, a1) = (k as f32 / n as f32 * tau, (k + 1) as f32 / n as f32 * tau);
            let (p0, p1) = (Vec3::new(a0.cos(), 0.0, a0.sin()) * radius, Vec3::new(a1.cos(), 0.0, a1.sin()) * radius);
            m.push_cylinder(p0, p1 - p0, 0.06, (p1 - p0).length(), 4, c, 1.0);
        }
    }
    // Spiral arms.
    for arm in 0..5 {
        let mut prev = None;
        for s in 0..=14 {
            let t = s as f32 / 14.0;
            let a = arm as f32 / 5.0 * tau + t * 2.4;
            let p = Vec3::new(a.cos(), 0.0, a.sin()) * (1.0 + t * 7.0) + Vec3::Y * (1.0 - t) * 0.6;
            if let Some(q) = prev {
                let d: Vec3 = p - q;
                m.push_cylinder(q, d, 0.05, d.length(), 4, c, 1.0);
            }
            prev = Some(p);
        }
    }
    m
}

/// The Giant Box: a lattice cube, BOX_SIZE wide and BOX_HEIGHT tall, with a
/// grid on its walls and roof and a solid floor.
fn box_mesh() -> MeshData {
    let mut m = MeshData::default();
    let c = Vec4::ONE;
    let (s, h) = (BOX_SIZE * 0.5, BOX_HEIGHT);
    let mut bar = |a: Vec3, b: Vec3, r: f32| {
        let d = b - a;
        m.push_cylinder(a, d, r, d.length(), 5, c, 1.0);
    };
    let corners = [Vec3::new(-s, 0.0, -s), Vec3::new(s, 0.0, -s), Vec3::new(s, 0.0, s), Vec3::new(-s, 0.0, s)];
    // Corner posts and the frame round the bottom and the roof.
    for k in 0..4 {
        let (a, b) = (corners[k], corners[(k + 1) % 4]);
        bar(a, a + Vec3::Y * h, 0.35);
        bar(a, b, 0.25);
        bar(a + Vec3::Y * h, b + Vec3::Y * h, 0.3);
        // The wall's grid.
        for i in 1..8 {
            let f = i as f32 / 8.0;
            let p = a.lerp(b, f);
            bar(p, p + Vec3::Y * h, 0.09);
        }
        for j in 1..5 {
            let y = Vec3::Y * h * j as f32 / 5.0;
            bar(a + y, b + y, 0.09);
        }
    }
    // The roof's grid.
    for i in 1..8 {
        let f = -s + 2.0 * s * i as f32 / 8.0;
        bar(Vec3::new(f, h, -s), Vec3::new(f, h, s), 0.09);
        bar(Vec3::new(-s, h, f), Vec3::new(s, h, f), 0.09);
    }
    m.push_box(Vec3::new(0.0, -0.15, 0.0), Vec3::new(BOX_SIZE, 0.3, BOX_SIZE), c, BoxUv::World(2.0));
    m
}

/// A blizzard: a wide ring of ice with snow streaks swirling inside.
fn storm_mesh() -> MeshData {
    let mut m = MeshData::default();
    let c = Vec4::ONE;
    let tau = std::f32::consts::TAU;
    let r = STORM_RADIUS;
    let n = 48;
    for (height, radius) in [(0.1f32, r), (4.0, r * 0.95), (8.0, r * 0.85)] {
        for k in 0..n {
            let (a0, a1) = (k as f32 / n as f32 * tau, (k + 1) as f32 / n as f32 * tau);
            let (p0, p1) = (Vec3::new(a0.cos() * radius, height, a0.sin() * radius), Vec3::new(a1.cos() * radius, height, a1.sin() * radius));
            m.push_cylinder(p0, p1 - p0, 0.06, (p1 - p0).length(), 4, c, 1.0);
        }
    }
    // Falling snow: slanted streaks scattered through the storm.
    for k in 0..90 {
        let h = |s: u32| ((k * 7919 + s * 104_729) % 1000) as f32 / 1000.0;
        let a = h(1) * tau;
        let d = h(2).sqrt() * r;
        let y = 0.5 + h(3) * 8.0;
        let p = Vec3::new(a.cos() * d, y, a.sin() * d);
        let dir = Vec3::new(0.5, -1.0, 0.2).normalize();
        m.push_cylinder(p, dir, 0.035, 0.9, 3, c, 1.0);
    }
    m
}

impl Structures {
    /// Blizzards: (centre, reach, rank, id).
    pub fn storms(&self) -> Vec<(Vec3, f32, f32, u32)> {
        self.list
            .iter()
            .filter(|b| b.s.kind == Kind::Storm)
            .map(|b| (b.s.pos, STORM_RADIUS * b.s.scale, b.s.power, b.s.id))
            .collect()
    }

    /// Healing from pads where you stand: (health per second, whether a
    /// Sanctuary pad also recharges shields).
    pub fn heal_at(&self, feet: Vec3) -> (f32, bool) {
        let mut out = (0.0, false);
        for b in self.list.iter().filter(|b| b.s.kind == Kind::HealPad) {
            let d = feet - b.s.pos;
            if Vec3::new(d.x, 0.0, d.z).length() < PAD_RADIUS * b.s.scale && d.y.abs() < 2.5 {
                out.0 += b.s.power;
                out.1 |= b.s.warden;
            }
        }
        out
    }

    /// Gravity wells: (centre, reach, strength, id).
    pub fn wells(&self) -> Vec<(Vec3, f32, f32, u32)> {
        self.list
            .iter()
            .filter(|b| b.s.kind == Kind::Well)
            .map(|b| (b.s.pos, WELL_RADIUS * b.s.scale, b.s.power, b.s.id))
            .collect()
    }

    pub fn new(renderer: &mut Renderer) -> Self {
        let mut pool = |n: usize, mesh: &MeshData, material: &MaterialData| -> Vec<usize> {
            (0..n)
                .map(|_| {
                    let h = renderer.add_dynamic_mesh(mesh, material);
                    renderer.set_transform(h, Mat4::IDENTITY, false);
                    h
                })
                .collect()
        };
        Self {
            list: Vec::new(),
            next: 1,
            dirty: false,
            barrier_handles: pool(MAX_BARRIERS, &barrier_mesh(), &glow(Vec4::new(0.6, 2.0, 3.4, 1.0))),
            ceiling_handles: pool(MAX_CEILINGS, &ceiling_mesh(), &glow(Vec4::new(2.6, 0.9, 3.2, 1.0))),
            dome_handles: pool(MAX_DOMES, &dome_mesh(), &glow(Vec4::new(1.8, 0.6, 3.4, 1.0))),
            pad_handles: pool(MAX_PADS, &pad_mesh(), &glow(Vec4::new(0.5, 3.0, 0.9, 1.0))),
            well_handles: pool(MAX_WELLS, &well_mesh(), &glow(Vec4::new(1.4, 0.4, 3.6, 1.0))),
            storm_handles: pool(MAX_STORMS, &storm_mesh(), &glow(Vec4::new(1.6, 2.8, 3.6, 1.0))),
            box_handles: pool(MAX_BOXES, &box_mesh(), &glow(Vec4::new(0.5, 1.8, 3.6, 1.0))),
            time: 0.0,
        }
    }

    #[cfg(test)]
    fn bare() -> Self {
        Self {
            list: Vec::new(),
            next: 1,
            dirty: false,
            barrier_handles: Vec::new(),
            ceiling_handles: Vec::new(),
            dome_handles: Vec::new(),
            pad_handles: Vec::new(),
            well_handles: Vec::new(),
            storm_handles: Vec::new(),
            box_handles: Vec::new(),
            time: 0.0,
        }
    }

    /// A new id for something you build (`me`: your player number).
    pub fn new_id(&mut self, me: usize) -> u32 {
        self.next += 1;
        ((me as u32 + 1) << 24) | (self.next & 0x00ff_ffff)
    }

    /// Adds a structure (yours or the other player's). Too many of a kind:
    /// the oldest goes.
    pub fn add(&mut self, s: Structure) {
        if self.list.iter().any(|b| b.s.id == s.id) {
            return;
        }
        let limit = match s.kind {
            Kind::Barrier => MAX_BARRIERS,
            Kind::Ceiling => MAX_CEILINGS,
            Kind::Dome => MAX_DOMES,
            Kind::HealPad => MAX_PADS,
            Kind::Well => MAX_WELLS,
            Kind::Storm => MAX_STORMS,
            Kind::Box => MAX_BOXES,
        };
        if self.list.iter().filter(|b| b.s.kind == s.kind).count() >= limit
            && let Some(i) = self.list.iter().position(|b| b.s.kind == s.kind)
        {
            self.list.remove(i);
        }
        self.list.push(Built { s, age: 0.0, flash: 0.0 });
        self.dirty = true;
    }

    /// Your own structures of a kind, oldest first.
    pub fn mine(&self, kind: Kind, me: usize) -> Vec<u32> {
        let tag = (me as u32 + 1) << 24;
        self.list.iter().filter(|b| b.s.kind == kind && b.s.id & 0xff00_0000 == tag).map(|b| b.s.id).collect()
    }

    pub fn remove(&mut self, id: u32) -> Option<Structure> {
        let i = self.list.iter().position(|b| b.s.id == id)?;
        self.dirty = true;
        Some(self.list.remove(i).s)
    }

    pub fn clear(&mut self) {
        self.dirty |= !self.list.is_empty();
        self.list.clear();
    }

    /// True once after the solid boxes changed.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    pub fn boxes(&self) -> Vec<Aabb> {
        self.list.iter().flat_map(|b| b.s.boxes()).collect()
    }

    pub fn blockers(&self) -> Blockers {
        Blockers {
            boxes: self.list.iter().flat_map(|b| b.s.boxes().into_iter().map(move |a| (a, b.s.id))).collect(),
            domes: self.list.iter().filter(|b| b.s.kind == Kind::Dome).map(|b| (b.s.pos, b.s.radius(b.age), b.s.id)).collect(),
        }
    }

    /// Damages a structure. True if it's now broken (take it away with
    /// [`Structures::remove`]).
    pub fn damage(&mut self, id: u32, amount: f32) -> bool {
        let Some(b) = self.list.iter_mut().find(|b| b.s.id == id) else { return false };
        b.s.health -= amount;
        b.flash = 1.0;
        b.s.health <= 0.0
    }

    /// Everything within `radius` of `center` (blasts).
    pub fn near(&self, center: Vec3, radius: f32) -> Vec<u32> {
        self.list
            .iter()
            .filter(|b| match b.s.kind {
                Kind::Dome => (center.distance(b.s.pos) - b.s.radius(b.age)).abs() < radius,
                Kind::HealPad | Kind::Well | Kind::Storm => false,
                _ => b.s.boxes().iter().any(|a| a.closest_point(center).distance(center) < radius),
            })
            .map(|b| b.s.id)
            .collect()
    }

    /// Structures a melee swing from `origin` along `dir` reaches.
    pub fn in_reach(&self, origin: Vec3, dir: Vec3, reach: f32) -> Vec<u32> {
        let mut ids: Vec<u32> = self.blockers().hit(origin, dir, reach).map(|(_, id)| id).into_iter().collect();
        for id in self.near(origin + dir * reach * 0.5, reach * 0.5) {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        ids
    }

    /// Ages everything; returns what timed out (fades quietly).
    pub fn update(&mut self, dt: f32) -> Vec<u32> {
        self.time += dt;
        for b in &mut self.list {
            b.age += dt;
            b.s.life -= dt;
            b.flash = (b.flash - dt * 5.0).max(0.0);
        }
        let gone: Vec<u32> = self.list.iter().filter(|b| b.s.life <= 0.0).map(|b| b.s.id).collect();
        for &id in &gone {
            self.remove(id);
        }
        gone
    }

    /// Keeps a player on the side of every dome shell they were on at
    /// `before` (their feet last frame): nobody leaves or enters a dome.
    /// Returns true if a dome held them.
    pub fn constrain(&self, player: &mut Player, before: Vec3) -> bool {
        let mut held = false;
        for b in self.list.iter().filter(|b| b.s.kind == Kind::Dome) {
            let c = b.s.pos;
            let r = b.s.radius(b.age);
            let mid_before = before + Vec3::Y * 0.9;
            let mid = player.pos + Vec3::Y * 0.9;
            let inside = mid_before.distance(c) < r;
            let to = mid - c;
            let d = to.length().max(1e-4);
            let dir = to / d;
            let margin = PLAYER_RADIUS + 0.05;
            if inside && d > r - margin {
                // Held in: back onto the inside of the shell. Squeezed if
                // the dome shrank past you.
                let limit = (r - margin).max(0.2);
                let mid = c + dir * limit;
                player.shove(mid - Vec3::Y * 0.9, -dir);
                held = true;
            } else if !inside && d < r + margin {
                player.shove(c + dir * (r + margin) - Vec3::Y * 0.9, dir);
                held = true;
            }
        }
        // Giant Boxes: whichever side of the walls you started the frame on,
        // you stay on (teleports, dashes and rewinds included).
        for b in self.list.iter().filter(|b| b.s.kind == Kind::Box) {
            let (c, half, h) = (b.s.pos, BOX_SIZE * b.s.scale * 0.5, BOX_HEIGHT * b.s.scale);
            let inside = |p: Vec3| (p.x - c.x).abs() < half && (p.z - c.z).abs() < half && p.y < c.y + h && p.y > c.y - 2.0;
            if inside(before) != inside(player.pos) {
                player.teleport(before);
                held = true;
            }
        }
        held
    }

    /// Health bars: (where, health fraction) for damaged structures.
    pub fn bars(&self) -> Vec<(Vec3, f32)> {
        self.list
            .iter()
            .filter(|b| b.s.health < b.s.max_health)
            .map(|b| (b.s.top(), b.s.health / b.s.max_health))
            .collect()
    }

    pub fn draw(&self, renderer: &mut Renderer) {
        let mut pools: [(Kind, &[usize], usize); 7] =
            [
            (Kind::Barrier, &self.barrier_handles, 0),
            (Kind::Ceiling, &self.ceiling_handles, 0),
            (Kind::Dome, &self.dome_handles, 0),
            (Kind::HealPad, &self.pad_handles, 0),
            (Kind::Well, &self.well_handles, 0),
            (Kind::Storm, &self.storm_handles, 0),
            (Kind::Box, &self.box_handles, 0),
        ];
        for b in &self.list {
            let Some(pool) = pools.iter_mut().find(|p| p.0 == b.s.kind) else { continue };
            let Some(&h) = pool.1.get(pool.2) else { continue };
            pool.2 += 1;
            let grow = (b.age / BUILD_TIME).clamp(0.0, 1.0);
            let ease = 1.0 - (1.0 - grow).powi(3);
            let pulse = 1.0 + 0.04 * b.flash;
            // Low on health or about to fade: it flickers.
            let weak = b.s.health < b.s.max_health * 0.3 || b.s.life < 2.0;
            let shown = !weak || (self.time * 14.0).sin() > -0.4;
            let t = match b.s.kind {
                Kind::Barrier => Mat4::from_scale_rotation_translation(
                    Vec3::new(
                        (if b.s.power > 0.0 { b.s.power / BARRIER_WIDTH } else { b.s.scale }) * pulse,
                        ease.max(0.02) * b.s.scale,
                        pulse * b.s.scale,
                    ),
                    Quat::from_rotation_y(-b.s.yaw - std::f32::consts::FRAC_PI_2),
                    b.s.pos,
                ),
                Kind::Ceiling => Mat4::from_scale_rotation_translation(
                    {
                        let (w, d) = b.s.ceiling_size();
                        Vec3::new(w / CEILING_SIZE * pulse, pulse, d / CEILING_SIZE * pulse)
                    },
                    Quat::IDENTITY,
                    b.s.pos + Vec3::Y * 3.0 * (1.0 - ease),
                ),
                Kind::Dome => Mat4::from_scale_rotation_translation(
                    Vec3::splat(b.s.radius(b.age) * ease.max(0.02) * pulse),
                    Quat::from_rotation_y(self.time * 0.2),
                    b.s.pos,
                ),
                Kind::HealPad => Mat4::from_scale_rotation_translation(
                    Vec3::splat(b.s.scale * ease.max(0.02) * (1.0 + 0.05 * (self.time * 4.0).sin())),
                    Quat::from_rotation_y(self.time * 0.6),
                    b.s.pos + Vec3::Y * 0.05,
                ),
                Kind::Well => Mat4::from_scale_rotation_translation(
                    Vec3::splat(b.s.scale * ease.max(0.02)),
                    Quat::from_rotation_y(-self.time * 2.5),
                    b.s.pos + Vec3::Y * 0.3,
                ),
                Kind::Box => Mat4::from_scale_rotation_translation(
                    Vec3::new(b.s.scale * pulse, b.s.scale * ease.max(0.02), b.s.scale * pulse),
                    Quat::IDENTITY,
                    b.s.pos,
                ),
                Kind::Storm => Mat4::from_scale_rotation_translation(
                    Vec3::splat(b.s.scale * ease.max(0.02)),
                    Quat::from_rotation_y(self.time * 1.4),
                    b.s.pos,
                ),
            };
            renderer.set_transform(h, t, shown);
        }
        for (_, handles, used) in pools {
            for &h in &handles[used.min(handles.len())..] {
                renderer.set_transform(h, Mat4::IDENTITY, false);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dome(at: Vec3) -> Structure {
        Structure { id: 7, kind: Kind::Dome, pos: at, yaw: 0.0, health: 100.0, max_health: 100.0, life: 10.0, warden: false, scale: 1.0, power: 0.0 }
    }

    #[test]
    fn domes_hold_players_in_and_keep_them_out() {
        let mut s = Structures::bare();
        s.add(dome(Vec3::ZERO));
        // Inside, walking out: held at the shell.
        let mut p = Player::new(Vec3::ZERO);
        let before = p.pos;
        p.pos = Vec3::new(9.0, 0.0, 0.0);
        assert!(s.constrain(&mut p, before));
        assert!((p.pos + Vec3::Y * 0.9).length() < DOME_RADIUS, "{:?}", p.pos);
        // Outside, walking in: kept out.
        let mut q = Player::new(Vec3::new(10.0, 0.0, 0.0));
        let before = q.pos;
        q.pos = Vec3::new(0.5, 0.0, 0.0);
        assert!(s.constrain(&mut q, before));
        assert!((q.pos + Vec3::Y * 0.9).length() > DOME_RADIUS);
        // Far away: untouched.
        let mut r = Player::new(Vec3::new(20.0, 0.0, 0.0));
        let at = r.pos;
        assert!(!s.constrain(&mut r, at));
    }

    #[test]
    fn giant_boxes_hold_players_even_through_teleports() {
        let mut s = Structures::bare();
        s.add(Structure { id: 9, kind: Kind::Box, ..dome(Vec3::ZERO) });
        // Walls, floor and roof, all solid.
        assert_eq!(s.boxes().len(), 6);
        assert!(s.blockers().hit(Vec3::new(0.0, 1.0, 0.0), Vec3::X, 100.0).is_some_and(|(t, _)| t < BOX_SIZE));
        assert!(s.blockers().hit(Vec3::new(0.0, 1.0, 0.0), Vec3::Y, 100.0).is_some(), "roofed");
        // Teleporting out from inside: put back.
        let mut p = Player::new(Vec3::ZERO);
        let before = p.pos;
        p.pos = Vec3::new(40.0, 0.0, 0.0);
        assert!(s.constrain(&mut p, before));
        assert_eq!(p.pos, before);
        // From outside, in: put back too.
        let outside = Vec3::new(40.0, 0.0, 0.0);
        let mut q = Player::new(outside);
        q.pos = Vec3::ZERO;
        assert!(s.constrain(&mut q, outside));
        assert_eq!(q.pos, outside);
    }

    #[test]
    fn shots_stop_at_barriers_and_dome_shells_from_either_side() {
        let mut s = Structures::bare();
        s.add(Structure { id: 3, kind: Kind::Barrier, pos: Vec3::new(10.0, 0.0, 0.0), yaw: 0.0, ..dome(Vec3::ZERO) });
        s.add(dome(Vec3::new(0.0, 0.0, 30.0)));
        let b = s.blockers();
        let (t, id) = b.hit(Vec3::new(0.0, 1.5, 0.0), Vec3::X, 50.0).unwrap();
        assert_eq!(id, 3);
        assert!((9.0..10.5).contains(&t), "{t}");
        assert!(b.hit(Vec3::new(0.0, 11.0, 0.0), Vec3::X, 50.0).is_none(), "over the top");
        // From inside the dome, outwards.
        let (t, id) = b.hit(Vec3::new(0.0, 1.0, 30.0), Vec3::X, 10.0).unwrap();
        assert_eq!(id, 7);
        assert!((t - (DOME_RADIUS * DOME_RADIUS - 1.0).sqrt()).abs() < 0.01);
        // Damage breaks it.
        assert!(!s.damage(7, 60.0));
        assert!(s.damage(7, 60.0));
        assert!(s.remove(7).is_some());
        assert!(s.blockers().domes.is_empty());
    }
}
