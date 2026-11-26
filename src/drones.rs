//! Drones: the enemies. Seven kinds, each with its own body, colour,
//! movement and attack, so you can read a fight at a glance:
//!
//! - **Hoverer** (red): the classic ball with a glowing ring. Circles and
//!   plinks single orbs.
//! - **Skitter** (green): a six-legged crawler that runs circles round you
//!   on the ground, spitting a fan of three.
//! - **Diver** (magenta): a dart that stalks above you, shudders, then
//!   rams you at speed. Sidestep the dive.
//! - **Sniper** (blue): a tall spire that paints you with a laser, locks
//!   on and fires one fast, heavy shot where you *were*. Keep moving.
//! - **Brute** (amber): a slow armoured block, hard to shove and hard to
//!   kill, that fires a wide spread of five.
//! - **Swarmer** (cyan): tiny, fragile and fast; comes in packs of four.
//! - **Mortar** (yellow): lobs shells in an arc; a ring on the ground shows
//!   where each will burst.
//!
//! Every attack is telegraphed: the drone's glow brightens and swells
//! while it winds up, so you have a moment to react. They respawn a while
//! after being destroyed; each one destroyed is worth experience.

use glam::{Mat4, Quat, Vec2, Vec3, Vec4};

use crate::collision::Aabb;
use crate::mesh::{BoxUv, MeshData};
use crate::model::MaterialData;
use crate::renderer::Renderer;
use crate::terrain::Terrain;

/// The size of a standard (Hoverer) drone; other kinds scale from it.
pub const DRONE_RADIUS: f32 = 0.7;
/// Shots count as hits this close to a standard drone's centre (bigger
/// than the drone itself, so fast movers can still land shots).
pub const HIT_RADIUS: f32 = DRONE_RADIUS + 0.4;
const RESPAWN: f32 = 25.0;
const ORBIT: f32 = 5.0;
const MAX_ORBS: usize = 40;
/// Ground rings shown under incoming mortar shells.
const MAX_MARKERS: usize = 8;
const BLEED_TIME: f32 = 4.0;
/// Mortar shells: gravity, flight time and blast radius.
const SHELL_GRAVITY: f32 = 14.0;
const SHELL_FLIGHT: f32 = 1.7;
const SHELL_SPLASH: f32 = 3.5;
/// The bird flock that hunts high fliers: how many birds, how long it
/// hunts before scattering, how fast it flies and how far its cloud spreads.
const FLOCK_SIZE: usize = 240;
const FLOCK_HUNT: f32 = 16.0;
const FLOCK_SCATTER: f32 = 3.0;
const FLOCK_SPEED: f32 = 24.0;
const FLOCK_SPREAD: f32 = 11.0;
/// Each bird pecks for this much, at most once a second.
const PECK_DAMAGE: f32 = 1.2;
/// A diver's ram: speed and how long it lasts.
const DIVE_SPEED: f32 = 30.0;
const DIVE_TIME: f32 = 0.7;

/// What a hit does besides its damage.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Effect {
    /// Bleed damage per second (for 4 s).
    pub bleed: f32,
    /// Burn damage per second, and for how long.
    pub burn: (f32, f32),
    /// Seconds stunned (no moving, no shooting).
    pub stun: f32,
    /// Shove, m/s.
    pub knock: Vec3,
    /// Shock: every Nth hit adds `shock_bonus` damage (and arcs to a
    /// drone nearby with `chain`).
    pub shock_every: u8,
    pub shock_bonus: f32,
    pub chain: bool,
    /// Damage multiplier on a target still at full health.
    pub crit: f32,
}

/// What kind of damage a number on screen shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Hit,
    Crit,
    Bleed,
    Burn,
    Shock,
    Stun,
}

/// A damage number to float up from where it happened.
#[derive(Clone, Copy, Debug)]
pub struct Popup {
    pub pos: Vec3,
    pub amount: f32,
    pub status: Status,
}

/// What one hit did.
#[derive(Clone, Copy, Debug, Default)]
pub struct HitResult {
    /// Where it was destroyed, if it was.
    pub killed: Option<Vec3>,
    /// A second drone finished off by the shock arcing to it.
    pub chain_kill: Option<Vec3>,
    pub dealt: f32,
    /// Status effects started (bleed, burn, stun, shock).
    pub effects: u32,
}

/// What the drones did this frame.
#[derive(Default)]
pub struct Tick {
    /// Drones finished off by bleeding or burning: (where, by the other player).
    pub kills: Vec<(Vec3, bool)>,
    /// Damage over time dealt by your effects (for lifesteal).
    pub dealt: f32,
    /// Damage their orbs (and rams) did to you.
    pub player_damage: f32,
    /// Damage from the rare high-altitude bird swarm.
    pub bird_damage: f32,
    /// Orbs fired (where from), for sounds.
    pub fired: Vec<Vec3>,
    /// Big attacks starting to wind up (where), for a warning sound.
    pub warnings: Vec<Vec3>,
    /// Mortar shells bursting: (where, radius), for explosions.
    pub impacts: Vec<(Vec3, f32)>,
    /// The bird flock just appeared here (for its sound).
    pub flock: Option<Vec3>,
    /// Damage done to decoys (your clones): (which, how much).
    pub decoy_hits: Vec<(usize, f32)>,
}

/// One bird of the flock.
struct Bird {
    pos: Vec3,
    vel: Vec3,
    /// Its place in the swirling cloud (a direction and how far out), and
    /// its wingbeat.
    slot: Vec3,
    flap: f32,
    /// Seconds before it can peck you again.
    peck: f32,
}

/// The murmuration: a vast cloud of birds swirling round a leader that
/// hunts you, then scatters.
struct Flock {
    leader: Vec3,
    heading: Vec3,
    birds: Vec<Bird>,
    /// Seconds hunting left; below zero it's scattering.
    life: f32,
}

/// The kinds of drone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Hoverer,
    Skitter,
    Diver,
    Sniper,
    Brute,
    Swarmer,
    Mortar,
}

/// How a kind of drone is built and fights.
struct Spec {
    health: f32,
    /// Size relative to a Hoverer.
    size: f32,
    /// How quickly it closes on where it wants to be.
    chase: f32,
    /// Its glow (linear, bright).
    glow: Vec3,
    /// Attacks from this close.
    range: f32,
    /// Seconds of visible wind-up before each attack.
    windup: f32,
    /// Seconds between attacks (plus up to 60% at random).
    reload: f32,
    shot_speed: f32,
    damage: f32,
    /// Orbs per attack, fanned out by `spread` radians.
    shots: u32,
    spread: f32,
    /// How much knockback moves it (1 = fully).
    knock: f32,
    /// Home height above the ground.
    lift: f32,
}

impl Kind {
    fn spec(self) -> Spec {
        let s = |health, size, chase, glow: [f32; 3], range, windup, reload, shot_speed, damage, shots, spread, knock, lift| Spec {
            health,
            size,
            chase,
            glow: Vec3::from(glow),
            range,
            windup,
            reload,
            shot_speed,
            damage,
            shots,
            spread,
            knock,
            lift,
        };
        match self {
            Kind::Hoverer => s(6.0, 1.0, 2.0, [2.8, 0.35, 0.25], 32.0, 0.55, 2.6, 20.0, 7.0, 1, 0.0, 1.0, 6.5),
            Kind::Skitter => s(4.0, 0.8, 3.2, [0.7, 2.8, 0.3], 26.0, 0.35, 2.2, 15.0, 4.0, 3, 0.16, 1.0, 0.5),
            Kind::Diver => s(5.0, 0.9, 3.0, [2.8, 0.3, 2.3], 22.0, 0.65, 3.4, DIVE_SPEED, 12.0, 0, 0.0, 1.0, 7.5),
            Kind::Sniper => s(7.0, 1.1, 0.8, [0.35, 1.5, 3.2], 60.0, 1.3, 4.5, 48.0, 14.0, 1, 0.0, 0.8, 11.0),
            Kind::Brute => s(26.0, 1.8, 0.9, [3.2, 1.4, 0.15], 30.0, 0.9, 4.2, 16.0, 6.0, 5, 0.22, 0.3, 3.0),
            Kind::Swarmer => s(2.0, 0.5, 4.5, [0.3, 2.6, 2.8], 24.0, 0.3, 1.8, 22.0, 3.0, 1, 0.0, 1.4, 5.5),
            Kind::Mortar => s(9.0, 1.15, 1.0, [2.9, 2.5, 0.9], 45.0, 1.0, 4.0, 0.0, 14.0, 1, 0.0, 0.6, 4.5),
        }
    }

    /// The threat level at which this kind starts appearing: the gentle
    /// ones from the start, the nastier ones as the game goes on.
    pub fn tier(self) -> u32 {
        match self {
            Kind::Hoverer | Kind::Skitter => 0,
            Kind::Swarmer | Kind::Diver => 1,
            Kind::Mortar | Kind::Sniper => 2,
            Kind::Brute => 3,
        }
    }

    /// Big, slow attacks get a warning sound as they wind up.
    fn warns(self) -> bool {
        matches!(self, Kind::Diver | Kind::Sniper | Kind::Brute | Kind::Mortar)
    }
}

struct Drone {
    kind: Kind,
    home: Vec3,
    pos: Vec3,
    /// Velocity last frame (facing, rams, the skitter's gait).
    vel: Vec3,
    /// The way its face points (smoothed).
    facing: Vec3,
    health: f32,
    /// Seconds until it comes back (when destroyed).
    respawn: f32,
    phase: f32,
    /// Its place in a swarm (0..4).
    member: f32,
    /// Time since last hit (drives the flinch and the faster circling).
    since_hit: f32,
    /// Where the host says it is (multiplayer joiner).
    net: Vec3,
    bleed: (f32, f32),
    burn: (f32, f32),
    stun: f32,
    knock: Vec3,
    shocks: u8,
    /// The other player hit it last (their kill if it bleeds out).
    remote: bool,
    /// Seconds until it may start its next attack.
    reload: f32,
    /// Seconds of wind-up left (0 = not attacking), and where it's aiming.
    windup: f32,
    aim: Vec3,
    /// A diver's ram: seconds left and direction.
    dive: f32,
    dive_dir: Vec3,
    /// Seconds before a ram can hurt you again.
    contact: f32,
    /// Damage over time not yet shown as a number.
    dot_shown: f32,
}

impl Drone {
    fn fresh(kind: Kind, home: Vec3, phase: f32, member: f32) -> Self {
        Drone {
            kind,
            home,
            pos: home,
            vel: Vec3::ZERO,
            facing: Vec3::X,
            health: kind.spec().health,
            respawn: 0.0,
            phase,
            member,
            since_hit: 10.0,
            net: home,
            bleed: (0.0, 0.0),
            burn: (0.0, 0.0),
            stun: 0.0,
            knock: Vec3::ZERO,
            shocks: 0,
            remote: false,
            reload: 2.0 + phase % 2.0,
            windup: 0.0,
            aim: home,
            dive: 0.0,
            dive_dir: Vec3::X,
            contact: 0.0,
            dot_shown: 0.0,
        }
    }

    fn alive(&self) -> bool {
        self.health > 0.0
    }

    fn max_health(&self) -> f32 {
        self.kind.spec().health
    }

    fn radius(&self) -> f32 {
        DRONE_RADIUS * self.kind.spec().size
    }

    fn clear_effects(&mut self) {
        self.bleed = (0.0, 0.0);
        self.burn = (0.0, 0.0);
        self.stun = 0.0;
        self.knock = Vec3::ZERO;
        self.shocks = 0;
        self.windup = 0.0;
        self.dive = 0.0;
    }
}

/// A drone's shot at you.
struct Orb {
    pos: Vec3,
    vel: Vec3,
    life: f32,
    damage: f32,
    /// Its colour (the drone's glow).
    color: Vec3,
    /// Mortar shells: they fall, and burst over `splash` metres around
    /// where they land (`target`).
    gravity: f32,
    splash: f32,
    target: Vec3,
}

/// A drone's state for its health bar: where, health left, and effects.
pub struct DroneBar {
    pub pos: Vec3,
    pub health: f32,
    pub bleeding: bool,
    pub burning: bool,
    pub stunned: bool,
}

/// The meshes one drone is drawn with.
struct Parts {
    /// The dark body and its glowing lights.
    shell: usize,
    glow: usize,
    /// A sniper's aiming laser.
    laser: Option<usize>,
}

pub struct Drones {
    drones: Vec<Drone>,
    handles: Vec<Parts>,
    time: f32,
    orbs: Vec<Orb>,
    orb_handles: Vec<usize>,
    marker_handles: Vec<usize>,
    birds: Option<Flock>,
    /// The whole flock, drawn as one group.
    bird_group: Option<usize>,
    bird_roll: f32,
    /// Damage numbers since last taken.
    pub popups: Vec<Popup>,
    /// Multiplier on how hard and how often drones shoot (0 = peaceful).
    pub aggression: f32,
    /// Structures that stop orbs, and orb hits on them since last taken.
    pub blockers: crate::structures::Blockers,
    pub struct_hits: Vec<(u32, f32, Vec3)>,
    /// Your clones' middles: drones attack whichever of you and them is
    /// nearest, and their shots hit clones too (see `Tick::decoy_hits`).
    pub decoys: Vec<Vec3>,
    /// The threat level: kinds of a higher [`Kind::tier`] stay away (out of
    /// the world) until it rises, then arrive one by one.
    pub threat: u32,
    /// The world's solid boxes (houses, walls, rocks): drones are pushed
    /// out of them, so they go round buildings instead of through them.
    pub walls: Vec<Aabb>,
}

/// A UV sphere.
pub fn sphere_mesh(radius: f32, rings: usize, segments: usize) -> MeshData {
    let mut m = MeshData::default();
    let point = |ring: usize, seg: usize| {
        let theta = ring as f32 / rings as f32 * std::f32::consts::PI;
        let phi = seg as f32 / segments as f32 * std::f32::consts::TAU;
        Vec3::new(theta.sin() * phi.cos(), theta.cos(), theta.sin() * phi.sin()) * radius
    };
    let uv = [Vec2::ZERO; 3];
    for r in 0..rings {
        for s in 0..segments {
            let (a, b, c, d) = (point(r, s), point(r, s + 1), point(r + 1, s + 1), point(r + 1, s));
            let out = (a + b + c + d) * 0.25;
            if r > 0 {
                m.push_tri([a, b, c], uv, Vec4::ONE, out);
            }
            if r + 1 < rings {
                m.push_tri([a, c, d], uv, Vec4::ONE, out);
            }
        }
    }
    m
}

/// Shape helpers for building the drone bodies.
mod shapes {
    use super::*;

    pub fn append(m: &mut MeshData, other: MeshData, color: Vec4) {
        let base = m.vertices.len() as u32;
        m.vertices.extend(other.vertices.into_iter().map(|mut v| {
            v.color = color.to_array();
            v
        }));
        m.indices.extend(other.indices.into_iter().map(|i| i + base));
    }

    /// An ellipsoid.
    pub fn ball(m: &mut MeshData, center: Vec3, scale: Vec3, color: Vec4) {
        let mut s = sphere_mesh(1.0, 8, 12);
        s.transform(Mat4::from_scale_rotation_translation(scale, Quat::IDENTITY, center));
        append(m, s, color);
    }

    /// A faceted cone from a round `base` to a point along `axis`.
    pub fn cone(m: &mut MeshData, base: Vec3, axis: Vec3, radius: f32, length: f32, color: Vec4) {
        let segments = 10;
        let a = axis.normalize();
        let u = a.any_orthonormal_vector();
        let v = a.cross(u);
        let tip = base + a * length;
        let uv = [Vec2::ZERO; 3];
        for i in 0..segments {
            let t = |k: u32| k as f32 / segments as f32 * std::f32::consts::TAU;
            let p0 = base + (u * t(i).cos() + v * t(i).sin()) * radius;
            let p1 = base + (u * t(i + 1).cos() + v * t(i + 1).sin()) * radius;
            let out = ((p0 + p1) * 0.5 - base).normalize_or_zero() * length + a * radius;
            m.push_tri([p0, p1, tip], uv, color, out);
            m.push_tri([p0, p1, base], uv, color, -a);
        }
    }

    pub fn rod(m: &mut MeshData, base: Vec3, axis: Vec3, radius: f32, length: f32, color: Vec4) {
        m.push_cylinder(base, axis, radius, length, 10, color, 1.0);
    }

    pub fn block(m: &mut MeshData, center: Vec3, size: Vec3, color: Vec4) {
        m.push_box(center, size, color, BoxUv::PerFace);
    }

    /// A flat triangle seen from both sides (fins, wings).
    pub fn fin(m: &mut MeshData, p: [Vec3; 3], color: Vec4) {
        let n = (p[1] - p[0]).cross(p[2] - p[0]);
        m.push_tri(p, [Vec2::ZERO; 3], color, n);
        m.push_tri(p, [Vec2::ZERO; 3], color, -n);
    }

    /// A stretched octahedron (a crystal shard).
    pub fn shard(m: &mut MeshData, center: Vec3, r: Vec3, color: Vec4) {
        let uv = [Vec2::ZERO; 3];
        for sx in [1.0, -1.0] {
            for sy in [1.0, -1.0] {
                for sz in [1.0, -1.0] {
                    let (x, y, z) = (Vec3::X * sx * r.x, Vec3::Y * sy * r.y, Vec3::Z * sz * r.z);
                    m.push_tri([center + x, center + y, center + z], uv, color, x + y + z);
                }
            }
        }
    }
}

/// The body (vertex-coloured, lit) and the glowing parts of a kind of
/// drone, at Hoverer scale facing +X.
fn build(kind: Kind) -> (MeshData, MeshData) {
    use shapes::*;
    let glow_tint = kind.spec().glow / kind.spec().glow.max_element();
    // Dark metal with a hint of the drone's colour, and lighter armour.
    let body = (Vec3::new(0.13, 0.14, 0.17) + glow_tint * 0.05).extend(1.0);
    let armour = (Vec3::new(0.22, 0.23, 0.27) + glow_tint * 0.05).extend(1.0);
    let lit = Vec4::ONE;
    let (mut shell, mut glow) = (MeshData::default(), MeshData::default());
    match kind {
        Kind::Hoverer => {
            ball(&mut shell, Vec3::ZERO, Vec3::splat(0.7), body);
            cone(&mut shell, Vec3::new(0.0, -0.5, 0.0), Vec3::NEG_Y, 0.32, 0.38, armour);
            rod(&mut shell, Vec3::new(0.0, 0.6, 0.0), Vec3::Y, 0.035, 0.45, armour);
            rod(&mut glow, Vec3::new(0.0, -0.06, 0.0), Vec3::Y, 0.78, 0.12, lit);
            ball(&mut glow, Vec3::new(0.6, 0.12, 0.0), Vec3::splat(0.2), lit);
            ball(&mut glow, Vec3::new(0.0, 1.08, 0.0), Vec3::splat(0.08), lit);
        }
        Kind::Skitter => {
            ball(&mut shell, Vec3::ZERO, Vec3::new(0.85, 0.38, 0.7), body);
            ball(&mut shell, Vec3::new(-0.2, 0.18, 0.0), Vec3::new(0.55, 0.25, 0.5), armour);
            for side in [1.0f32, -1.0] {
                for x in [-0.45, 0.0, 0.45] {
                    let base = Vec3::new(x, 0.0, side * 0.45);
                    let out = Vec3::new(x * 0.6, -0.9, side).normalize();
                    rod(&mut shell, base, out, 0.055, 0.85, armour);
                }
                cone(&mut shell, Vec3::new(0.72, -0.08, side * 0.18), Vec3::new(1.0, -0.2, side * 0.3), 0.08, 0.32, armour);
                ball(&mut glow, Vec3::new(0.72, 0.12, side * 0.18), Vec3::splat(0.11), lit);
            }
            block(&mut glow, Vec3::new(-0.15, 0.42, 0.0), Vec3::new(0.8, 0.04, 0.14), lit);
        }
        Kind::Diver => {
            cone(&mut shell, Vec3::new(-0.1, 0.0, 0.0), Vec3::X, 0.42, 1.05, body);
            cone(&mut shell, Vec3::new(-0.1, 0.0, 0.0), Vec3::NEG_X, 0.42, 0.45, body);
            fin(&mut shell, [Vec3::new(-0.2, 0.2, 0.0), Vec3::new(-0.75, 0.7, 0.0), Vec3::new(-0.55, 0.1, 0.0)], armour);
            for side in [1.0f32, -1.0] {
                fin(&mut shell, [Vec3::new(0.1, 0.0, side * 0.3), Vec3::new(-0.7, 0.0, side * 0.95), Vec3::new(-0.5, 0.0, side * 0.3)], armour);
            }
            rod(&mut glow, Vec3::new(-0.5, 0.0, 0.0), Vec3::NEG_X, 0.2, 0.1, lit);
            ball(&mut glow, Vec3::new(0.42, 0.13, 0.0), Vec3::new(0.24, 0.07, 0.2), lit);
        }
        Kind::Sniper => {
            rod(&mut shell, Vec3::new(0.0, -0.9, 0.0), Vec3::Y, 0.26, 1.45, body);
            cone(&mut shell, Vec3::new(0.0, -0.9, 0.0), Vec3::NEG_Y, 0.26, 0.55, armour);
            ball(&mut shell, Vec3::new(0.0, 0.62, 0.0), Vec3::splat(0.38), armour);
            rod(&mut shell, Vec3::new(0.25, 0.62, 0.0), Vec3::X, 0.07, 1.35, body);
            ball(&mut glow, Vec3::new(0.33, 0.72, 0.0), Vec3::new(0.1, 0.12, 0.17), lit);
            for y in [-0.45, 0.05] {
                rod(&mut glow, Vec3::new(0.0, y, 0.0), Vec3::Y, 0.31, 0.06, lit);
            }
            rod(&mut glow, Vec3::new(1.55, 0.62, 0.0), Vec3::X, 0.11, 0.07, lit);
        }
        Kind::Brute => {
            block(&mut shell, Vec3::ZERO, Vec3::new(1.2, 1.0, 1.2), body);
            block(&mut shell, Vec3::new(0.62, 0.0, 0.0), Vec3::new(0.12, 0.85, 1.05), armour);
            for side in [1.0f32, -1.0] {
                block(&mut shell, Vec3::new(-0.05, 0.25, side * 0.75), Vec3::new(0.95, 0.6, 0.3), armour);
                rod(&mut shell, Vec3::new(0.3, -0.28, side * 0.45), Vec3::X, 0.12, 0.72, body);
                rod(&mut glow, Vec3::new(1.0, -0.28, side * 0.45), Vec3::X, 0.13, 0.05, lit);
            }
            block(&mut glow, Vec3::new(0.69, 0.22, 0.0), Vec3::new(0.04, 0.14, 0.72), lit);
            for (x, z) in [(0.3, 0.3), (0.3, -0.3), (-0.3, 0.3), (-0.3, -0.3)] {
                block(&mut glow, Vec3::new(x, 0.51, z), Vec3::new(0.22, 0.03, 0.22), lit);
            }
        }
        Kind::Swarmer => {
            shard(&mut shell, Vec3::ZERO, Vec3::new(0.75, 0.42, 0.42), body);
            for side in [1.0f32, -1.0] {
                fin(&mut shell, [Vec3::new(0.15, 0.0, side * 0.3), Vec3::new(-0.35, 0.15, side * 1.05), Vec3::new(-0.5, 0.0, side * 0.3)], armour);
            }
            ball(&mut glow, Vec3::new(0.5, 0.0, 0.0), Vec3::splat(0.15), lit);
            ball(&mut glow, Vec3::new(-0.65, 0.0, 0.0), Vec3::splat(0.11), lit);
        }
        Kind::Mortar => {
            ball(&mut shell, Vec3::ZERO, Vec3::new(0.75, 0.5, 0.75), body);
            cone(&mut shell, Vec3::new(0.0, -0.28, 0.0), Vec3::NEG_Y, 0.62, 0.35, armour);
            let tube = Vec3::new(0.4, 1.0, 0.0).normalize();
            rod(&mut shell, Vec3::new(0.0, 0.15, 0.0), tube, 0.2, 0.85, armour);
            rod(&mut glow, Vec3::new(0.0, 0.15, 0.0) + tube * 0.85, tube, 0.23, 0.05, lit);
            rod(&mut glow, Vec3::new(0.0, -0.06, 0.0), Vec3::Y, 0.78, 0.08, lit);
        }
    }
    (shell, glow)
}

/// Cheap deterministic hash to scatter the drones.
fn hash(i: u32, salt: u32) -> f32 {
    let mut x = i.wrapping_mul(747_796_405).wrapping_add(salt.wrapping_mul(2_891_336_453));
    x = ((x >> ((x >> 28) + 4)) ^ x).wrapping_mul(277_803_737);
    ((x >> 22) ^ x) as f32 / u32::MAX as f32
}

/// Every drone and its home: a few gentle ones near the start, then a mix
/// of every kind scattered over the valley (swarmers in packs of four).
/// Deterministic, so every player in a game builds the same list.
fn roster(terrain: &Terrain, start: Vec3) -> Vec<(Kind, Vec3, f32)> {
    use Kind::*;
    let place = |kind: Kind, x: f32, z: f32| {
        let ground = terrain.height_at(x, z).max(terrain.water_level);
        Vec3::new(x, ground + kind.spec().lift, z)
    };
    let mut out: Vec<(Kind, Vec3, f32)> = [(Hoverer, 14.0, -10.0), (Skitter, -16.0, 6.0), (Hoverer, 6.0, 18.0)]
        .iter()
        .map(|&(kind, dx, dz)| (kind, place(kind, start.x + dx, start.z + dz), 0.0))
        .collect();
    const MIX: [Kind; 13] = [Hoverer, Skitter, Sniper, Diver, Swarmer, Hoverer, Mortar, Skitter, Brute, Diver, Sniper, Swarmer, Mortar];
    let reach = terrain.half_size - 30.0;
    for i in 0..20u32 {
        let x = (hash(i, 1) * 2.0 - 1.0) * reach;
        let z = (hash(i, 2) * 2.0 - 1.0) * reach;
        let kind = MIX[i as usize % MIX.len()];
        if kind == Swarmer {
            for k in 0..4 {
                let a = k as f32 * std::f32::consts::FRAC_PI_2;
                out.push((kind, place(kind, x + a.cos() * 2.5, z + a.sin() * 2.5), k as f32));
            }
        } else {
            out.push((kind, place(kind, x, z), 0.0));
        }
    }
    out
}

/// Moves a ball of radius `r` at `pos` out of any box it overlaps: to just
/// off the nearest surface (or, if its centre is inside, out through the
/// nearest face).
fn push_out(pos: &mut Vec3, r: f32, walls: &[Aabb]) {
    for w in walls {
        let c = w.closest_point(*pos);
        let off = *pos - c;
        let dist2 = off.length_squared();
        if dist2 >= r * r {
            continue;
        }
        if dist2 > 1e-8 {
            *pos = c + off / dist2.sqrt() * r;
        } else {
            // Inside: the nearest face wins.
            let (lo, hi) = (*pos - w.min, w.max - *pos);
            let faces = [(lo.x, -Vec3::X), (hi.x, Vec3::X), (lo.y, -Vec3::Y), (hi.y, Vec3::Y), (lo.z, -Vec3::Z), (hi.z, Vec3::Z)];
            let (depth, dir) = faces.into_iter().fold((f32::MAX, Vec3::Y), |best, f| if f.0 < best.0 { f } else { best });
            *pos += dir * (depth + r);
        }
    }
}

fn glow(x: f32) -> Vec4 {
    Vec4::new(x, x, x, 1.0)
}

/// Rotation turning +X to face `dir` (yaw, then pitch; no roll).
fn face(dir: Vec3) -> Quat {
    let yaw = (-dir.z).atan2(dir.x);
    let pitch = dir.y.clamp(-1.0, 1.0).asin();
    Quat::from_rotation_y(yaw) * Quat::from_rotation_z(pitch)
}

impl Drones {
    pub fn new(renderer: &mut Renderer, terrain: &Terrain, start: Vec3) -> Self {
        let shell_material = MaterialData {
            base_color: Vec4::ONE,
            specular: 1.2,
            shininess: 60.0,
            ground_occlusion: 0.0,
            ..Default::default()
        };
        let drones: Vec<Drone> = roster(terrain, start)
            .into_iter()
            .enumerate()
            .map(|(i, (kind, home, member))| Drone::fresh(kind, home, i as f32 * 1.7, member))
            .collect();
        let mut laser = MeshData::default();
        laser.push_cylinder(Vec3::ZERO, Vec3::Z, 0.035, 1.0, 6, Vec4::ONE, 1.0);
        let handles = drones
            .iter()
            .map(|d| {
                let (shell, lights) = build(d.kind);
                let glow_material = MaterialData { base_color: d.kind.spec().glow.extend(1.0), ground_occlusion: 0.0, ..Default::default() };
                Parts {
                    shell: renderer.add_dynamic_mesh(&shell, &shell_material),
                    glow: renderer.add_dynamic_mesh(&lights, &glow_material),
                    laser: (d.kind == Kind::Sniper).then(|| renderer.add_dynamic_mesh(&laser, &glow_material)),
                }
            })
            .collect();
        let orb = sphere_mesh(0.22, 6, 8);
        let orb_material = MaterialData { base_color: Vec4::new(1.3, 1.3, 1.3, 1.0), ground_occlusion: 0.0, ..Default::default() };
        let hidden = |renderer: &mut Renderer, mesh: &MeshData, material: &MaterialData| {
            let h = renderer.add_dynamic_mesh(mesh, material);
            renderer.set_transform(h, Mat4::IDENTITY, false);
            h
        };
        let orb_handles = (0..MAX_ORBS).map(|_| hidden(renderer, &orb, &orb_material)).collect();
        let mut ring = MeshData::default();
        ring.push_cylinder(Vec3::ZERO, Vec3::Y, 1.0, 0.04, 28, Vec4::ONE, 1.0);
        let marker_material = MaterialData { base_color: Vec4::new(2.6, 1.6, 0.3, 1.0), ground_occlusion: 0.0, ..Default::default() };
        let marker_handles = (0..MAX_MARKERS).map(|_| hidden(renderer, &ring, &marker_material)).collect();
        // A bird facing +X: a slim body, swept wings along ±Z, a forked tail.
        let mut bird_mesh = MeshData::default();
        let dark = Vec4::new(0.55, 0.5, 0.6, 1.0);
        shapes::fin(&mut bird_mesh, [Vec3::new(0.5, 0.0, 0.0), Vec3::new(-0.35, 0.06, 0.0), Vec3::new(-0.35, -0.06, 0.0)], dark);
        for side in [1.0f32, -1.0] {
            shapes::fin(&mut bird_mesh, [Vec3::new(0.2, 0.0, 0.0), Vec3::new(-0.25, 0.04, side * 0.85), Vec3::new(-0.15, 0.0, 0.0)], Vec4::ONE);
            shapes::fin(&mut bird_mesh, [Vec3::new(-0.3, 0.0, 0.0), Vec3::new(-0.6, 0.0, side * 0.2), Vec3::new(-0.45, 0.0, 0.0)], dark);
        }
        let bird_material = MaterialData { base_color: Vec4::new(1.1, 0.75, 0.35, 1.0), ground_occlusion: 0.0, ..Default::default() };
        let bird_group = Some(renderer.add_dynamic_group(&bird_mesh, &bird_material, FLOCK_SIZE));
        Self {
            drones,
            handles,
            time: 0.0,
            orbs: Vec::new(),
            orb_handles,
            marker_handles,
            birds: None,
            bird_group,
            bird_roll: 73.0,
            popups: Vec::new(),
            aggression: 1.0,
            blockers: Default::default(),
            struct_hits: Vec::new(),
            decoys: Vec::new(),
            threat: u32::MAX,
            walls: Vec::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn test_at(positions: &[Vec3]) -> Self {
        Self::test_kinds(&positions.iter().map(|&p| (Kind::Hoverer, p)).collect::<Vec<_>>())
    }

    #[cfg(test)]
    pub(crate) fn test_kinds(list: &[(Kind, Vec3)]) -> Self {
        let drones = list.iter().map(|&(kind, p)| Drone::fresh(kind, p, 0.0, 0.0)).collect();
        Self {
            drones,
            handles: Vec::new(),
            time: 0.0,
            orbs: Vec::new(),
            orb_handles: Vec::new(),
            marker_handles: Vec::new(),
            birds: None,
            bird_group: None,
            bird_roll: 73.0,
            popups: Vec::new(),
            aggression: 0.0,
            blockers: Default::default(),
            struct_hits: Vec::new(),
            decoys: Vec::new(),
            threat: u32::MAX,
            walls: Vec::new(),
        }
    }

    /// Health bars for drones that were hit recently or carry an effect.
    pub fn bars(&self) -> Vec<DroneBar> {
        self.drones
            .iter()
            .filter(|d| d.alive() && (d.since_hit < 4.0 || d.bleed.1 > 0.0 || d.burn.1 > 0.0 || d.stun > 0.0))
            .map(|d| DroneBar {
                // Above bigger drones' heads too.
                pos: d.pos + Vec3::Y * (d.radius() - DRONE_RADIUS).max(0.0) * 1.3,
                health: d.health / d.max_health(),
                bleeding: d.bleed.1 > 0.0,
                burning: d.burn.1 > 0.0,
                stunned: d.stun > 0.0,
            })
            .collect()
    }

    fn popup(&mut self, pos: Vec3, amount: f32, status: Status) {
        if self.popups.len() < 64 {
            self.popups.push(Popup { pos, amount, status });
        }
    }

    #[cfg(test)]
    pub fn alive_count(&self) -> usize {
        self.drones.iter().filter(|d| d.alive()).count()
    }

    /// Every drone's position and whether it's up (host -> joiner).
    pub fn states(&self) -> Vec<(Vec3, bool)> {
        self.drones.iter().map(|d| (d.pos, d.alive())).collect()
    }

    /// A joiner takes the host's drones. Returns where drones just went down.
    pub fn apply_states(&mut self, states: &[(Vec3, bool)]) -> Vec<Vec3> {
        let mut downed = Vec::new();
        for (d, &(pos, alive)) in self.drones.iter_mut().zip(states) {
            if d.alive() && !alive {
                downed.push(d.pos);
                d.health = 0.0;
                d.respawn = RESPAWN;
                d.clear_effects();
            } else if !d.alive() && alive {
                d.health = d.max_health();
                d.pos = pos;
            }
            d.net = pos;
        }
        downed
    }

    /// Moves the drones and draws them. `dt` is world time (slowed by Time
    /// powers). Without `simulate` (a multiplayer joiner) they glide to
    /// where the host says they are instead.
    ///
    /// Every player's drones shoot at them (`target`: their middle, None
    /// when they can't be hurt) with orbs only they see; bleeding and
    /// burning tick only where drones are simulated.
    pub fn update(&mut self, renderer: &mut Renderer, dt: f32, player: Vec3, simulate: bool, target: Option<Vec3>, terrain: &Terrain) -> Tick {
        let tick = self.simulate(dt, player, simulate, target, terrain);
        self.draw(renderer);
        tick
    }

    #[cfg(test)]
    pub fn tick_effects_for_test(&mut self, dt: f32, terrain: &Terrain) -> Vec<(Vec3, bool)> {
        self.simulate(dt, Vec3::splat(1000.0), true, None, terrain).kills
    }

    #[cfg(test)]
    pub fn tick_for_test(&mut self, dt: f32, player: Vec3, terrain: &Terrain) -> Tick {
        self.simulate(dt, player, true, Some(player + Vec3::Y * 0.9), terrain)
    }

    fn draw(&self, renderer: &mut Renderer) {
        for (i, d) in self.drones.iter().enumerate() {
            let Some(parts) = self.handles.get(i) else { continue };
            let spec = d.kind.spec();
            let charge = if d.windup > 0.0 { 1.0 - d.windup / spec.windup } else { 0.0 };
            let flinch = 1.0 + (0.25 - d.since_hit).max(0.0) * 1.2 * spec.knock.min(1.0);
            let mut rot = face(d.facing);
            if d.stun > 0.0 {
                rot *= Quat::from_rotation_x(0.5 + (self.time * 30.0 + i as f32).sin() * 0.3);
            } else if d.kind == Kind::Hoverer && d.windup <= 0.0 {
                // Hoverers idly turn their ring.
                rot *= Quat::from_rotation_y((self.time * 1.5 + i as f32).sin() * 0.6);
            }
            // A skitter's bobbing gait; everything else bobs gently in the air.
            let bob = if d.kind == Kind::Skitter {
                (self.time * 18.0 + i as f32).sin().abs() * 0.07 * d.vel.length().min(1.0)
            } else {
                (self.time * 2.0 + i as f32).sin() * 0.06
            };
            // Shaking just before a diver rams.
            let shake = if d.kind == Kind::Diver && charge > 0.4 {
                Vec3::new((self.time * 70.0).sin(), (self.time * 53.0).cos(), 0.0) * 0.06 * charge
            } else {
                Vec3::ZERO
            };
            let swell = 1.0 + 0.14 * charge * charge;
            let t = Mat4::from_scale_rotation_translation(Vec3::splat(flinch * swell * spec.size), rot, d.pos + Vec3::Y * bob + shake);
            let flash = (0.12 - d.since_hit).max(0.0) / 0.12;
            let lights = if d.stun > 0.0 {
                glow(0.25 + 0.35 * (self.time * 40.0).sin().abs())
            } else {
                glow(1.0 + 3.0 * charge * charge + 2.0 * flash)
            };
            renderer.set_transform_tinted(parts.shell, t, glow(1.0 + 5.0 * flash), d.alive());
            renderer.set_transform_tinted(parts.glow, t, lights, d.alive());
            if let Some(laser) = parts.laser {
                // The sniper's laser: thin while it tracks, thicker and
                // flickering once it's about to fire.
                let show = d.alive() && d.windup > 0.0 && (charge < 0.7 || (self.time * 45.0).sin() > -0.3);
                let from = d.pos + rot * (Vec3::new(1.6, 0.62, 0.0) * spec.size);
                let to = d.aim - from;
                let width = 1.0 + charge * 1.5;
                let t = Mat4::from_scale_rotation_translation(
                    Vec3::new(width, width, to.length() + 6.0),
                    Quat::from_rotation_arc(Vec3::Z, to.normalize_or(Vec3::Z)),
                    from,
                );
                renderer.set_transform_tinted(laser, t, glow(0.5 + 1.8 * charge), show);
            }
        }
        for (i, &h) in self.orb_handles.iter().enumerate() {
            match self.orbs.get(i) {
                Some(o) => {
                    let pulse = (1.0 + 0.25 * (self.time * 25.0 + i as f32).sin()) * if o.splash > 0.0 { 1.8 } else { 1.0 };
                    let t = Mat4::from_scale_rotation_translation(Vec3::splat(pulse), Quat::IDENTITY, o.pos);
                    renderer.set_transform_tinted(h, t, (o.color * 0.8).extend(1.0), true);
                }
                None => renderer.set_transform(h, Mat4::IDENTITY, false),
            }
        }
        // Where mortar shells will land: a ring that tightens as they fall.
        let mut shells = self.orbs.iter().filter(|o| o.splash > 0.0);
        for &h in &self.marker_handles {
            match shells.next() {
                Some(o) => {
                    let close = (1.0 - o.life / (SHELL_FLIGHT + 1.0)).clamp(0.0, 1.0);
                    let r = o.splash * (1.3 - 0.3 * close);
                    let t = Mat4::from_scale_rotation_translation(Vec3::new(r, 1.0, r), Quat::IDENTITY, o.target + Vec3::Y * 0.06);
                    renderer.set_transform_tinted(h, t, glow(0.6 + 0.8 * (self.time * 12.0).sin().abs()), true);
                }
                None => renderer.set_transform(h, Mat4::IDENTITY, false),
            }
        }
        if let Some(group) = self.bird_group {
            let birds: Vec<crate::renderer::Instance> = self
                .birds
                .iter()
                .flat_map(|f| f.birds.iter())
                .enumerate()
                .map(|(i, b)| {
                    // Wingbeats: the span folds and spreads, and the bird
                    // banks into its turns.
                    let beat = b.flap.sin();
                    let size = 1.4 + hash(i as u32, 7) * 0.9;
                    let scale = Vec3::new(size, size, size * (0.55 + 0.45 * beat.abs()));
                    let dir = b.vel.normalize_or(Vec3::X);
                    let bank = Quat::from_rotation_x(beat * 0.25);
                    let shade = 0.75 + 0.5 * hash(i as u32, 8);
                    crate::renderer::Instance {
                        transform: Mat4::from_scale_rotation_translation(scale, face(dir) * bank, b.pos),
                        tint: glow(shade),
                    }
                })
                .collect();
            renderer.set_instances(group, &birds);
        }
    }

    fn simulate(&mut self, dt: f32, player: Vec3, simulate: bool, target: Option<Vec3>, terrain: &Terrain) -> Tick {
        self.time += dt;
        let time = self.time;
        let mut tick = Tick::default();
        let mut popups = Vec::new();
        let mut fire: Vec<Orb> = Vec::new();
        if simulate {
            self.birds_update(dt, player, target, terrain, &mut tick);
        }
        let me = player + Vec3::Y * 1.2;
        for (i, d) in self.drones.iter_mut().enumerate() {
            let spec = d.kind.spec();
            let before = d.pos;
            d.since_hit += dt;
            d.stun = (d.stun - dt).max(0.0);
            d.contact = (d.contact - dt).max(0.0);
            if !simulate {
                d.pos += (d.net - d.pos) * (dt * 10.0).min(1.0);
            }
            // Too dangerous for the threat level yet: it leaves quietly (no
            // kill, no blast) until the threat rises.
            let waiting = d.kind.tier() > self.threat;
            if simulate && d.alive() && waiting {
                d.health = 0.0;
                d.respawn = 1.0;
                d.clear_effects();
            }
            if simulate && !d.alive() {
                d.respawn -= dt;
                if waiting {
                    // Arrives a moment after the threat rises, not all at once.
                    d.respawn = d.respawn.max(1.0 + hash(i as u32, 51) * 6.0);
                } else if d.respawn <= 0.0 {
                    d.health = d.max_health();
                    d.pos = d.home;
                    d.clear_effects();
                }
            }
            if simulate && d.alive() {
                // Bleeding and burning.
                let mut dot = 0.0;
                for (dps, left) in [&mut d.bleed, &mut d.burn] {
                    if *left > 0.0 {
                        dot += *dps * dt.min(*left);
                        *left -= dt;
                    }
                }
                if dot > 0.0 {
                    d.health -= dot;
                    d.dot_shown += dot;
                    if !d.remote {
                        tick.dealt += dot;
                    }
                    if d.dot_shown >= 1.0 || !d.alive() {
                        let status = if d.burn.1 > 0.0 { Status::Burn } else { Status::Bleed };
                        popups.push(Popup { pos: d.pos, amount: d.dot_shown, status });
                        d.dot_shown = 0.0;
                    }
                    if !d.alive() {
                        tick.kills.push((d.pos, d.remote));
                        d.respawn = RESPAWN;
                        d.clear_effects();
                        continue;
                    }
                }
                if d.stun <= 0.0 {
                    let spin = if d.since_hit < 2.0 { 1.6 } else { 0.45 };
                    d.phase += dt * spin;
                }
                if d.stun > 0.0 {
                    // Stunned: sags and wobbles in place.
                    d.dive = 0.0;
                    d.pos += Vec3::new((time * 23.0).sin() * 0.05, -0.6 * dt, 0.0);
                } else if d.dive > 0.0 {
                    d.dive -= dt;
                    d.pos += d.dive_dir * DIVE_SPEED * dt;
                } else {
                    let goal = Self::goal(d, i, me, time, terrain);
                    d.pos += (goal - d.pos) * (dt * spec.chase).min(1.0);
                }
            }
            if d.alive() {
                // Knockback (also seen by a joiner, ahead of the host's word).
                d.pos += d.knock * dt;
                d.knock *= (-4.0 * dt).exp();
                let floor = terrain.height_at(d.pos.x, d.pos.z) + d.radius() * 0.75;
                d.pos.y = d.pos.y.max(floor);
                let r = d.radius();
                push_out(&mut d.pos, r, &self.walls);
            }
            if dt > 0.0 {
                d.vel = d.vel.lerp((d.pos - before) / dt, (dt * 10.0).min(1.0));
            }

            // Attacking this player: wind up (visibly), then strike.
            let in_reach = target.filter(|_| d.alive() && d.stun <= 0.0 && self.aggression > 0.0);
            if let Some(you) = in_reach {
                let t = self.decoys.iter().copied().fold(you, |best, c| if c.distance(d.pos) < best.distance(d.pos) { c } else { best });
                let to = t - d.pos;
                let dist = to.length();
                if d.windup > 0.0 {
                    d.windup -= dt * self.aggression.sqrt();
                    // Snipers lock on; everything else tracks you.
                    if d.kind != Kind::Sniper {
                        d.aim = t;
                    }
                    if d.windup <= 0.0 {
                        d.windup = 0.0;
                        d.reload = spec.reload * (1.0 + 0.6 * hash(i as u32, (time * 10.0) as u32));
                        Self::strike(d, &spec, terrain, &mut fire);
                    }
                } else if d.dive <= 0.0 {
                    d.reload -= dt * self.aggression.sqrt();
                    let provoked = d.since_hit < 8.0 || dist < spec.range * 0.6 || matches!(d.kind, Kind::Sniper | Kind::Mortar);
                    // A joiner doesn't move the drones, so it can't run a diver's ram.
                    let can = d.kind != Kind::Diver || simulate;
                    if d.reload <= 0.0 && dist < spec.range && provoked && can {
                        d.windup = spec.windup;
                        d.aim = t;
                        if d.kind.warns() {
                            tick.warnings.push(d.pos);
                        }
                    }
                }
                // A diver ramming through you.
                if d.kind == Kind::Diver && d.contact <= 0.0 && d.vel.length() > 14.0 && you.distance(d.pos) < d.radius() + 0.9 {
                    tick.player_damage += spec.damage * self.aggression;
                    d.contact = 1.0;
                    d.dive = 0.0;
                    d.knock = -d.vel * 0.4;
                }
            } else {
                d.windup = 0.0;
            }

            // Facing: towards you while aiming, else the way it's going.
            let aiming = d.windup > 0.0 || (matches!(d.kind, Kind::Sniper | Kind::Brute | Kind::Mortar) && target.is_some_and(|t| t.distance(d.pos) < spec.range));
            let mut want = if aiming {
                target.map_or(d.aim, |t| if d.kind == Kind::Sniper && d.windup > 0.0 { d.aim } else { t }) - d.pos
            } else if d.vel.length() > 0.5 {
                d.vel
            } else {
                d.facing
            };
            if d.dive > 0.0 {
                want = d.dive_dir;
            }
            if !matches!(d.kind, Kind::Diver | Kind::Swarmer) {
                want.y = 0.0;
            }
            let want = want.normalize_or(d.facing);
            d.facing = d.facing.lerp(want, (dt * 6.0).min(1.0)).normalize_or(Vec3::X);
        }
        self.popups.extend(popups);
        for o in fire {
            if self.orbs.len() < MAX_ORBS {
                tick.fired.push(o.pos);
                self.orbs.push(o);
            }
        }
        self.orbs_update(dt, target, terrain, &mut tick);
        if target.is_none() {
            self.orbs.clear();
        }
        tick
    }

    /// Where a drone wants to be this frame.
    fn goal(d: &Drone, i: usize, me: Vec3, time: f32, terrain: &Terrain) -> Vec3 {
        let bob = (time * 1.3 + i as f32).sin() * 0.6;
        let circle = |r: f32| Vec3::new(d.phase.cos() * r, bob, d.phase.sin() * r);
        let dist = (me - d.pos).length();
        let flat_away = |from: Vec3| {
            let v = from - me;
            Vec3::new(v.x, 0.0, v.z).normalize_or(Vec3::X)
        };
        match d.kind {
            Kind::Hoverer => {
                // Curious: drift partway towards a nearby player.
                let mut g = d.home + circle(ORBIT);
                let to = me + Vec3::Y * 1.3 - g;
                if to.length() < 30.0 {
                    g += to * 0.35;
                }
                g
            }
            Kind::Skitter => {
                let mut g = if dist < 30.0 {
                    // Runs circles round you, zig-zagging.
                    let a = d.phase * 1.5;
                    let r = 7.0 + (time * 3.0 + i as f32).sin() * 1.5;
                    me + Vec3::new(a.cos() * r, 0.0, a.sin() * r)
                } else {
                    d.home + circle(ORBIT)
                };
                g.y = terrain.height_at(g.x, g.z) + 0.45;
                g
            }
            Kind::Diver => {
                if dist < 38.0 {
                    me + Vec3::Y * 4.5 + circle(6.0)
                } else {
                    d.home + circle(ORBIT)
                }
            }
            Kind::Sniper => d.home + Vec3::Y * ((time * 0.7 + i as f32).sin() * 0.4),
            Kind::Brute => {
                if dist < 35.0 {
                    me + flat_away(d.home) * 10.0 + Vec3::Y * 1.8
                } else {
                    d.home + circle(2.0)
                }
            }
            Kind::Swarmer => {
                let a = d.phase * 2.2 + d.member * std::f32::consts::FRAC_PI_2;
                let wobble = (time * 3.0 + d.member).sin() * 1.2;
                if dist < 28.0 {
                    me + Vec3::new(a.cos() * 5.0, 1.5 + wobble, a.sin() * 5.0)
                } else {
                    d.home + Vec3::new(a.cos() * 2.2, wobble * 0.5, a.sin() * 2.2)
                }
            }
            Kind::Mortar => {
                // Keeps its distance.
                if dist < 20.0 {
                    d.pos + flat_away(d.pos) * 4.0
                } else {
                    d.home + Vec3::Y * bob * 0.5
                }
            }
        }
    }

    /// A drone's wound-up attack goes off.
    fn strike(d: &mut Drone, spec: &Spec, terrain: &Terrain, fire: &mut Vec<Orb>) {
        let color = spec.glow;
        let from = d.pos + d.facing * d.radius();
        match d.kind {
            Kind::Diver => {
                d.dive = DIVE_TIME;
                d.dive_dir = (d.aim - d.pos).normalize_or(Vec3::NEG_Y);
            }
            Kind::Mortar => {
                // Lobbed to land at your feet.
                let feet = d.aim - Vec3::Y * 0.9;
                let land = Vec3::new(feet.x, feet.y.max(terrain.height_at(feet.x, feet.z)), feet.z);
                let vel = (land - from) / SHELL_FLIGHT + Vec3::Y * 0.5 * SHELL_GRAVITY * SHELL_FLIGHT;
                fire.push(Orb {
                    pos: from,
                    vel,
                    life: SHELL_FLIGHT + 1.0,
                    damage: spec.damage,
                    color,
                    gravity: SHELL_GRAVITY,
                    splash: SHELL_SPLASH,
                    target: land,
                });
            }
            _ => {
                let dir = (d.aim - from).normalize_or(Vec3::X);
                let n = spec.shots.max(1);
                for k in 0..n {
                    let turn = (k as f32 - (n - 1) as f32 * 0.5) * spec.spread;
                    let dir = Quat::from_rotation_y(turn) * dir;
                    fire.push(Orb {
                        pos: from,
                        vel: dir * spec.shot_speed,
                        life: 4.0,
                        damage: spec.damage,
                        color,
                        gravity: 0.0,
                        splash: 0.0,
                        target: Vec3::ZERO,
                    });
                }
            }
        }
    }

    /// A vast flock that now and then hunts players flying high: it
    /// sweeps in from the distance, swoops through you again and again
    /// (every bird you meet pecks), then scatters into the sky.
    fn birds_update(&mut self, dt: f32, player: Vec3, target: Option<Vec3>, terrain: &Terrain, tick: &mut Tick) {
        let height = player.y - terrain.height_at(player.x, player.z);
        if self.birds.is_none() {
            self.bird_roll -= dt;
            if height > 8.0 && self.bird_roll <= 0.0 && hash(self.time.to_bits(), 91) < dt * 0.003 {
                self.birds = Some(Self::new_flock(player, self.time.to_bits()));
                tick.flock = self.birds.as_ref().map(|f| f.leader);
                self.bird_roll = 70.0 + hash(self.time.to_bits(), 92) * 100.0;
            }
        }
        let (time, aggression) = (self.time, self.aggression);
        let Some(f) = &mut self.birds else { return };
        f.life -= dt;
        let hunting = f.life > 0.0;
        let aim = target.unwrap_or(player + Vec3::Y);
        // The leader swoops at you, overshoots and loops back round; when
        // the hunt is over it climbs away.
        let want = if hunting { (aim - f.leader).normalize_or(f.heading) } else { (f.heading + Vec3::Y).normalize() };
        f.heading = (f.heading + (want - f.heading) * (dt * 1.6).min(1.0)).normalize_or(want);
        f.leader += f.heading * FLOCK_SPEED * dt;
        f.leader.y = f.leader.y.max(terrain.height_at(f.leader.x, f.leader.z) + 4.0);
        // The cloud turns slowly, breathes in and out, and bursts apart as
        // it scatters.
        let swirl = Quat::from_rotation_y(time * 0.8);
        let breathe = 1.0 + 0.3 * (time * 0.9).sin();
        let spread = if hunting { breathe } else { breathe + -f.life * 4.0 };
        for (i, b) in f.birds.iter_mut().enumerate() {
            let k = i as f32;
            let wobble = Vec3::new((time * 1.3 + k).sin(), (time * 1.7 + k * 0.7).sin() * 0.6, (time * 1.1 + k * 1.3).cos()) * 1.5;
            let goal = f.leader + swirl * (b.slot * spread) + wobble;
            let desired = f.heading * FLOCK_SPEED + (goal - b.pos) * 2.5;
            b.vel = b.vel.lerp(desired, (dt * 3.0).min(1.0));
            b.pos += b.vel * dt;
            b.pos.y = b.pos.y.max(terrain.height_at(b.pos.x, b.pos.z) + 1.0);
            b.flap += dt * (11.0 + (i % 5) as f32);
            b.peck = (b.peck - dt).max(0.0);
            if hunting && target.is_some() && aggression > 0.0 && b.peck <= 0.0 && b.pos.distance(aim) < 1.6 {
                tick.bird_damage += PECK_DAMAGE * aggression.min(1.5);
                b.peck = 1.0;
            }
        }
        if f.life < -FLOCK_SCATTER {
            self.birds = None;
        }
    }

    /// A flock gathering 60 m away (from a random side, up high), its birds
    /// spread through a wide, flattened cloud.
    fn new_flock(player: Vec3, seed: u32) -> Flock {
        let side = hash(seed, 93) * std::f32::consts::TAU;
        let leader = player + Vec3::new(side.cos(), 0.0, side.sin()) * 60.0 + Vec3::Y * 12.0;
        let heading = (player - leader).normalize_or(Vec3::X);
        let birds = (0..FLOCK_SIZE as u32)
            .map(|i| {
                let a = hash(i, 11) * std::f32::consts::TAU;
                let up = hash(i, 12) * 2.0 - 1.0;
                let out = (1.0 - up * up).sqrt();
                let slot = Vec3::new(out * a.cos(), up * 0.45, out * a.sin()) * hash(i, 13).cbrt() * FLOCK_SPREAD;
                Bird { pos: leader + slot, vel: heading * FLOCK_SPEED, slot, flap: hash(i, 14) * std::f32::consts::TAU, peck: 0.0 }
            })
            .collect();
        Flock { leader, heading, birds, life: FLOCK_HUNT }
    }

    /// Orbs fly in world time; they burst on structures, the ground or you.
    /// Mortar shells burst over an area.
    fn orbs_update(&mut self, dt: f32, target: Option<Vec3>, terrain: &Terrain, tick: &mut Tick) {
        let aggression = self.aggression;
        let blockers = &self.blockers;
        let struct_hits = &mut self.struct_hits;
        let decoys = &self.decoys;
        self.orbs.retain_mut(|o| {
            o.vel.y -= o.gravity * dt;
            let step = o.vel * dt;
            let burst = |at: Vec3, tick: &mut Tick| {
                if o.splash > 0.0 {
                    tick.impacts.push((at, o.splash));
                    if let Some(t) = target {
                        let d = t.distance(at);
                        if d < o.splash + 0.5 {
                            tick.player_damage += o.damage * aggression * (1.0 - 0.5 * (d / o.splash).min(1.0));
                        }
                    }
                    for (k, c) in decoys.iter().enumerate() {
                        let d = c.distance(at);
                        if d < o.splash + 0.5 {
                            tick.decoy_hits.push((k, o.damage * aggression * (1.0 - 0.5 * (d / o.splash).min(1.0))));
                        }
                    }
                }
            };
            if step != Vec3::ZERO
                && let Some((t, id)) = blockers.hit(o.pos, step.normalize(), step.length())
            {
                let at = o.pos + step.normalize() * t;
                struct_hits.push((id, o.damage, at));
                burst(at, tick);
                return false;
            }
            o.pos += step;
            o.life -= dt;
            if let Some(t) = target
                && o.pos.distance(t) < 0.9
            {
                if o.splash > 0.0 {
                    burst(o.pos, tick);
                } else {
                    tick.player_damage += o.damage * aggression;
                }
                return false;
            }
            if let Some(k) = decoys.iter().position(|c| o.pos.distance(*c) < 0.9) {
                if o.splash > 0.0 {
                    burst(o.pos, tick);
                } else {
                    tick.decoy_hits.push((k, o.damage * aggression));
                }
                return false;
            }
            let ground = terrain.height_at(o.pos.x, o.pos.z);
            if o.pos.y <= ground || o.life <= 0.0 {
                burst(Vec3::new(o.pos.x, o.pos.y.max(ground), o.pos.z), tick);
                return false;
            }
            true
        });
    }

    /// The nearest drone hit by a segment from `origin` along unit `dir`
    /// within `max`: (index, distance).
    pub fn ray_hit(&self, origin: Vec3, dir: Vec3, max: f32) -> Option<(usize, f32)> {
        self.ray_hits(origin, dir, max).into_iter().next()
    }

    /// Every drone along the segment, nearest first.
    pub fn ray_hits(&self, origin: Vec3, dir: Vec3, max: f32) -> Vec<(usize, f32)> {
        let mut hits: Vec<(usize, f32)> = self
            .drones
            .iter()
            .enumerate()
            .filter(|(_, d)| d.alive())
            .filter_map(|(i, d)| {
                let to = d.pos - origin;
                let along = to.dot(dir);
                let closest = (to - dir * along).length_squared();
                let r = d.radius() + (HIT_RADIUS - DRONE_RADIUS);
                let r2 = r * r;
                if closest > r2 {
                    return None;
                }
                let t = along - (r2 - closest).sqrt();
                (t >= 0.0 && t <= max).then_some((i, t))
            })
            .collect();
        hits.sort_by(|a, b| a.1.total_cmp(&b.1));
        hits
    }

    /// Shoves a drone (Pull, Repulse, Ground Pull, wells) and stuns it.
    pub fn push(&mut self, index: usize, knock: Vec3, stun: f32) {
        if let Some(d) = self.drones.get_mut(index).filter(|d| d.alive()) {
            d.knock += knock * d.kind.spec().knock;
            d.stun = d.stun.max(stun);
        }
    }

    /// Destroys drone shots within `radius` of `center` (Repulse).
    pub fn clear_orbs_near(&mut self, center: Vec3, radius: f32) -> usize {
        let before = self.orbs.len();
        self.orbs.retain(|o| o.pos.distance(center) > radius);
        before - self.orbs.len()
    }

    /// Where a living drone is.
    pub fn position(&self, index: usize) -> Option<Vec3> {
        self.drones.get(index).filter(|d| d.alive()).map(|d| d.pos)
    }

    /// Drones within `radius` of `center` (counting their size): (index, distance).
    pub fn within(&self, center: Vec3, radius: f32) -> Vec<(usize, f32)> {
        self.drones
            .iter()
            .enumerate()
            .filter(|(_, d)| d.alive())
            .map(|(i, d)| (i, d.pos.distance(center), d.radius()))
            .filter(|&(_, dist, r)| dist < radius + r)
            .map(|(i, dist, _)| (i, dist))
            .collect()
    }

    /// Damages a drone; returns where it was if this destroyed it.
    #[cfg(test)]
    pub fn damage(&mut self, index: usize, amount: f32) -> Option<Vec3> {
        self.hit(index, amount, &Effect::default(), false).killed
    }

    /// A hit with its effects: crits, shocks (which may arc to a drone
    /// nearby), bleeding, burning, stun and knockback. `remote`: the other
    /// player's.
    pub fn hit(&mut self, index: usize, amount: f32, e: &Effect, remote: bool) -> HitResult {
        let mut r = HitResult::default();
        let Some(d) = self.drones.get_mut(index) else { return r };
        if !d.alive() {
            return r;
        }
        let mut amount = amount;
        let mut status = Status::Hit;
        if e.crit > 1.0 && d.health >= d.max_health() - 1e-3 {
            amount *= e.crit;
            status = Status::Crit;
        }
        let mut arc = None;
        if e.shock_every > 0 {
            d.shocks += 1;
            if d.shocks >= e.shock_every {
                d.shocks = 0;
                amount += e.shock_bonus;
                status = Status::Shock;
                r.effects += 1;
                if e.chain {
                    arc = Some((d.pos, e.shock_bonus));
                }
            }
        }
        if e.bleed > 0.0 {
            if d.bleed.1 <= 0.0 {
                r.effects += 1;
            }
            d.bleed = (d.bleed.0.max(e.bleed), BLEED_TIME);
        }
        if e.burn.0 > 0.0 {
            if d.burn.1 <= 0.0 {
                r.effects += 1;
            }
            d.burn = (d.burn.0.max(e.burn.0), d.burn.1.max(e.burn.1));
        }
        if e.stun > 0.0 {
            if d.stun <= 0.0 {
                r.effects += 1;
            }
            d.stun = d.stun.max(e.stun);
            d.windup = 0.0;
        }
        d.knock += e.knock * d.kind.spec().knock;
        d.remote = remote;
        d.health -= amount;
        d.since_hit = 0.0;
        r.dealt = amount;
        let pos = d.pos;
        if !d.alive() {
            d.respawn = RESPAWN;
            d.clear_effects();
            r.killed = Some(pos);
        }
        self.popup(pos, amount, status);
        if e.stun > 0.0 && status == Status::Hit {
            self.popup(pos + Vec3::Y * 0.4, 0.0, Status::Stun);
        }
        // The shock jumps to the nearest other drone.
        if let Some((from, bonus)) = arc
            && let Some((j, _)) = self
                .within(from, 8.0)
                .into_iter()
                .filter(|&(j, _)| j != index)
                .min_by(|a, b| a.1.total_cmp(&b.1))
        {
            let plain = Effect { shock_every: 0, ..Default::default() };
            let chained = self.hit(j, bonus, &plain, remote);
            r.dealt += chained.dealt;
            if let Some(p) = self.popups.last_mut() {
                p.status = Status::Shock;
            }
            r.chain_kill = chained.killed;
        }
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rays_find_drones_in_order_and_damage_destroys() {
        let mut d = Drones::test_at(&[Vec3::new(10.0, 0.0, 0.0), Vec3::new(5.0, 0.2, 0.0), Vec3::new(5.0, 5.0, 0.0)]);
        let hits = d.ray_hits(Vec3::ZERO, Vec3::X, 50.0);
        assert_eq!(hits.iter().map(|h| h.0).collect::<Vec<_>>(), vec![1, 0]);
        assert!((hits[0].1 - (5.0 - (HIT_RADIUS * HIT_RADIUS - 0.04).sqrt())).abs() < 1e-4);
        assert!(d.ray_hit(Vec3::ZERO, Vec3::X, 3.0).is_none(), "out of range");
        assert!(d.damage(1, 4.0).is_none());
        assert!(d.damage(1, 4.0).is_some(), "destroyed");
        assert_eq!(d.alive_count(), 2);
        assert!(d.ray_hits(Vec3::ZERO, Vec3::X, 50.0).iter().all(|h| h.0 != 1), "dead drones aren't hit");
        assert_eq!(d.within(Vec3::new(5.0, 4.0, 0.0), 1.0).len(), 1);
    }

    #[test]
    fn kinds_differ_in_toughness_and_size() {
        let mut d = Drones::test_kinds(&[(Kind::Brute, Vec3::new(10.0, 2.0, 0.0)), (Kind::Swarmer, Vec3::new(20.0, 2.0, 0.0))]);
        assert!(d.damage(0, 10.0).is_none(), "brutes soak hits");
        assert!(d.damage(1, 2.0).is_some(), "swarmers pop");
        // A brute is big enough to hit with a ray that would miss a hoverer.
        assert!(d.ray_hit(Vec3::new(0.0, 3.3, 0.0), Vec3::X, 50.0).is_some());
    }

    #[test]
    fn attacks_are_telegraphed_before_they_land() {
        let flat = Terrain::flat(200.0);
        let player = Vec3::new(0.0, 0.0, 0.0);
        let mut d = Drones::test_kinds(&[(Kind::Sniper, Vec3::new(25.0, 8.0, 0.0))]);
        d.aggression = 1.0;
        d.drones[0].reload = 0.0;
        let tick = d.tick_for_test(1.0 / 60.0, player, &flat);
        assert!(tick.fired.is_empty(), "no instant shots");
        assert_eq!(tick.warnings.len(), 1, "the sniper warns as it locks on");
        assert!(d.drones[0].windup > 0.0);
        // Stepping aside during the lock-on dodges the shot.
        let aside = Vec3::new(0.0, 0.0, 6.0);
        let mut fired = false;
        let mut hurt = 0.0;
        for _ in 0..240 {
            let t = d.tick_for_test(1.0 / 60.0, aside, &flat);
            fired |= !t.fired.is_empty();
            hurt += t.player_damage;
        }
        assert!(fired, "it fires after winding up");
        assert_eq!(hurt, 0.0, "at where you were");
    }

    #[test]
    fn the_flock_is_vast_and_pecks_as_it_swoops_through() {
        let flat = Terrain::flat(400.0);
        let mut d = Drones::test_kinds(&[]);
        d.aggression = 1.0;
        let player = Vec3::new(0.0, 20.0, 0.0);
        d.birds = Some(Drones::new_flock(player, 5));
        assert_eq!(d.birds.as_ref().map(|f| f.birds.len()), Some(FLOCK_SIZE));
        let mut hurt = 0.0;
        for _ in 0..((FLOCK_HUNT + FLOCK_SCATTER + 1.0) * 60.0) as usize {
            hurt += d.tick_for_test(1.0 / 60.0, player, &flat).bird_damage;
        }
        assert!(hurt > 3.0, "it reaches you and pecks: {hurt}");
        assert!(d.birds.is_none(), "then scatters and leaves");
    }

    #[test]
    fn drones_shoot_the_nearer_clone_instead_of_you() {
        let flat = Terrain::flat(200.0);
        let mut d = Drones::test_kinds(&[(Kind::Hoverer, Vec3::new(10.0, 3.0, 0.0))]);
        d.aggression = 1.0;
        d.drones[0].reload = 0.0;
        d.drones[0].since_hit = 0.0; // provoked
        let you = Vec3::new(-15.0, 0.0, 0.0);
        let clone = Vec3::new(4.0, 0.9, 0.0);
        d.decoys = vec![clone];
        let (mut clone_hurt, mut you_hurt) = (0.0, 0.0);
        for _ in 0..240 {
            let t = d.tick_for_test(1.0 / 60.0, you, &flat);
            clone_hurt += t.decoy_hits.iter().map(|h| h.1).sum::<f32>();
            you_hurt += t.player_damage;
        }
        assert!(clone_hurt > 0.0, "the clone draws its fire");
        assert_eq!(you_hurt, 0.0);
    }

    #[test]
    fn nastier_drones_wait_for_the_threat_to_rise() {
        let flat = Terrain::flat(200.0);
        let mut d = Drones::test_kinds(&[(Kind::Hoverer, Vec3::new(50.0, 6.0, 0.0)), (Kind::Brute, Vec3::new(-50.0, 6.0, 0.0))]);
        d.threat = 0;
        d.tick_effects_for_test(1.0 / 60.0, &flat);
        assert_eq!(d.alive_count(), 1, "only the hoverer at first");
        d.threat = 3;
        for _ in 0..(8 * 60) {
            d.tick_effects_for_test(1.0 / 60.0, &flat);
        }
        assert_eq!(d.alive_count(), 2, "the brute arrives once the threat is high enough");
    }

    #[test]
    fn drones_go_round_houses_not_through_them() {
        let mut pos = Vec3::new(0.2, 1.0, 0.0);
        let house = [Aabb { min: Vec3::new(-2.0, 0.0, -2.0), max: Vec3::new(2.0, 3.0, 2.0) }];
        push_out(&mut pos, 0.5, &house);
        let probe = Aabb::from_center(pos, Vec3::splat(0.9));
        assert!(!probe.intersects(&house[0]), "pushed out: {pos:?}");
        let mut beside = Vec3::new(2.3, 1.0, 0.0);
        push_out(&mut beside, 0.5, &house);
        assert!((beside.x - 2.5).abs() < 1e-4, "nudged off the wall: {beside:?}");
    }

    #[test]
    fn mortar_shells_burst_where_you_stood() {
        let flat = Terrain::flat(200.0);
        let mut d = Drones::test_kinds(&[(Kind::Mortar, Vec3::new(30.0, 5.0, 0.0))]);
        d.aggression = 1.0;
        d.drones[0].reload = 0.0;
        let mut impacts = Vec::new();
        let mut hurt = 0.0;
        for _ in 0..300 {
            let t = d.tick_for_test(1.0 / 60.0, Vec3::ZERO, &flat);
            impacts.extend(t.impacts);
            hurt += t.player_damage;
        }
        assert!(!impacts.is_empty(), "a shell burst");
        assert!(impacts[0].0.length() < 1.5, "near your feet: {:?}", impacts[0].0);
        assert!(hurt > 0.0, "standing still gets you hit");
    }
}
