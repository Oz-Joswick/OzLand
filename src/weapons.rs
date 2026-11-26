//! Weapons (the Arsenal tree): Blaster, Scatter Gun, Rocket Launcher and
//! Railgun. Shots fly in world time, so Bullet Time slows them and Time
//! Stop freezes them in the air. Rocket blasts shove you as well as the
//! drones: aim at your feet for a rocket jump.

use glam::{Mat4, Quat, Vec2, Vec3, Vec4};

use crate::abilities::Moveset;
use crate::arena::{PVP_SCALE, body_distance, segment_hits_body};
use crate::collision::Aabb;
use crate::drones::{DRONE_RADIUS, Drones, Effect, sphere_mesh};
use crate::structures::{Blockers, SHOT_SCALE};
use crate::gadgets::raycast;
use crate::mesh::{BoxUv, MeshData};
use crate::model::MaterialData;
use crate::renderer::Renderer;
use crate::terrain::Terrain;

const MAX_SHOTS: usize = 96;
const MAX_ROCKETS: usize = 16;
const MAX_BLASTS: usize = 12;
const MAX_BEAMS: usize = 14;
const SHOT_LIFE: f32 = 2.5;
/// Blasts at least this big (m) dent the ground: meteors, fireballs,
/// cluster grenades and upgraded rockets, not ordinary rockets (6 m).
pub const CRATER_BLAST: f32 = 7.0;
/// Seeking bolts: how far they look for a target, how fast they turn
/// (per second), and how fast they fly.
const HOMING_RANGE: f32 = 45.0;
const HOMING_TURN: f32 = 7.0;
const HOMING_SPEED: f32 = 34.0;
const BLAST_TIME: f32 = 0.3;
const BEAM_TIME: f32 = 0.2;
const RAIL_RANGE: f32 = 250.0;
/// How hard a blast at point-blank range shoves you, m/s.
const BLAST_PUSH: f32 = 24.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Weapon {
    Blade,
    Hammer,
    Scythe,
    Blaster,
    Scatter,
    Rockets,
    Railgun,
}

/// Upgrades that change what a weapon's hits do, packed into a byte so the
/// other player's shots carry them too.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods(pub u8);

impl Mods {
    pub const BLEED: u8 = 1;
    pub const CONCUSSION: u8 = 2;
    pub const ION: u8 = 4;
    pub const SLUG: u8 = 8;
    pub const NAPALM: u8 = 16;
    pub const OVERCHARGE: u8 = 32;
    pub const MASTER: u8 = 64;
    pub const BLADE_CONE: u8 = 128;

    pub fn from_moves(m: &Moveset) -> Self {
        let mut b = 0;
        for (on, bit) in [
            (m.bleed, Self::BLEED),
            (m.concussion, Self::CONCUSSION),
            (m.ion_coil, Self::ION),
            (m.slug_shells, Self::SLUG),
            (m.napalm, Self::NAPALM),
            (m.overcharge, Self::OVERCHARGE),
            (m.effect_power > 1.0, Self::MASTER),
            (m.blade_cone, Self::BLADE_CONE),
        ] {
            if on {
                b |= bit;
            }
        }
        Mods(b)
    }

    pub fn has(self, bit: u8) -> bool {
        self.0 & bit != 0
    }

    /// Multiplier on status effects.
    fn power(self) -> f32 {
        if self.has(Self::MASTER) { 2.0 } else { 1.0 }
    }
}

impl Weapon {
    pub const ALL: [Weapon; 7] =
        [Weapon::Blade, Weapon::Hammer, Weapon::Scythe, Weapon::Blaster, Weapon::Scatter, Weapon::Rockets, Weapon::Railgun];

    pub fn name(self) -> &'static str {
        match self {
            Weapon::Blade => "Blade",
            Weapon::Hammer => "Warhammer",
            Weapon::Scythe => "Scythe",
            Weapon::Blaster => "Blaster",
            Weapon::Scatter => "Scatter Gun",
            Weapon::Rockets => "Rocket Launcher",
            Weapon::Railgun => "Railgun",
        }
    }

    pub fn is_melee(self) -> bool {
        matches!(self, Weapon::Blade | Weapon::Hammer | Weapon::Scythe)
    }

    /// How far a melee swing reaches.
    pub fn reach(self) -> f32 {
        self.swing().0
    }

    /// A melee hit's damage at weapon power 1.
    pub fn swing_damage(self) -> f32 {
        self.swing().2
    }

    /// Melee reach (m), arc (half-angle, radians) and damage per hit (at
    /// weapon power 1).
    fn swing(self) -> (f32, f32, f32) {
        match self {
            Weapon::Blade => (4.2, 0.9, 3.0),
            Weapon::Hammer => (5.0, 1.2, 7.0),
            Weapon::Scythe => (6.2, 1.65, 5.0),
            _ => (0.0, 0.0, 0.0),
        }
    }

    /// The damage effect, in a word (for the menu).
    pub fn effect(self) -> &'static str {
        match self {
            Weapon::Blade => "Fast; bleeds with Keen Edge",
            Weapon::Hammer => "Knockback and stun",
            Weapon::Scythe => "Wide sweep, bleed, heals you",
            Weapon::Blaster => "Shock every few hits",
            Weapon::Scatter => "Knockback",
            Weapon::Rockets => "Burns; blast shoves you",
            Weapon::Railgun => "Pierces; crits fresh targets",
        }
    }

    /// Its number in [`Weapon::ALL`] (sent over the network).
    pub fn index(self) -> u8 {
        Weapon::ALL.iter().position(|w| *w == self).unwrap_or(0) as u8
    }

    pub fn from_index(i: u8) -> Option<Weapon> {
        Weapon::ALL.get(i as usize).copied()
    }

    /// The key that selects it.
    pub fn key(self) -> &'static str {
        match self {
            Weapon::Blade => "1",
            Weapon::Hammer => "2",
            Weapon::Scythe => "3",
            Weapon::Blaster => "4",
            Weapon::Scatter => "5",
            Weapon::Rockets => "6",
            Weapon::Railgun => "7",
        }
    }

    /// For the menu: (damage per hit, hits per second, its effect).
    pub fn stats(self, moves: &Moveset) -> (String, f32, &'static str) {
        let d = moves.weapon_damage;
        let rate = self.rate(moves) / self.delay();
        let damage = match self {
            Weapon::Blade | Weapon::Hammer | Weapon::Scythe => format!("{:.1}", self.swing().2 * d * moves.melee_damage),
            Weapon::Blaster => format!("{d:.1}"),
            Weapon::Scatter => format!("{} x {d:.1}", moves.pellets),
            Weapon::Rockets => format!("{:.1} area", 6.0 * d),
            Weapon::Railgun => format!("{:.1}", 8.0 * d),
        };
        (damage, rate, self.effect())
    }

    /// Speed-up of the swing or reload.
    fn rate(self, moves: &Moveset) -> f32 {
        if self.is_melee() { moves.melee_speed } else { moves.fire_rate }
    }

    /// Seconds between shots at normal fire rate.
    fn delay(self) -> f32 {
        match self {
            Weapon::Blade => 0.32,
            Weapon::Hammer => 0.85,
            Weapon::Scythe => 0.5,
            Weapon::Blaster => 0.15,
            Weapon::Scatter => 0.9,
            Weapon::Rockets => 0.9,
            Weapon::Railgun => 1.1,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Bolt,
    Rocket,
}

struct Shot {
    pos: Vec3,
    vel: Vec3,
    kind: Kind,
    weapon: Weapon,
    mods: Mods,
    damage: f32,
    /// Blast radius, for rockets.
    blast: f32,
    life: f32,
    /// Fired by the other player (in multiplayer).
    remote: bool,
    /// Curves towards the nearest target ahead.
    homing: bool,
}

struct Blast {
    pos: Vec3,
    radius: f32,
    age: f32,
}

struct Beam {
    from: Vec3,
    to: Vec3,
    age: f32,
}

/// A melee swing's glowing arc.
struct Slash {
    origin: Vec3,
    dir: Vec3,
    reach: f32,
    arc: f32,
    age: f32,
    /// A Keen Edge slash travels forward instead of sweeping in place.
    wave: bool,
    duration: f32,
}

/// Keen Edge's carried blade cut. This is a single moving slash volume, not
/// a bundle of independent shots.
struct BladeWave {
    pos: Vec3,
    dir: Vec3,
    damage: f32,
    mods: Mods,
    remote: bool,
    life: f32,
    hit: Vec<usize>,
}

const SLASH_TIME: f32 = 0.16;
const SLASH_SEGMENTS: usize = 9;
const MAX_SLASHES: usize = 4;

/// One weapon firing, with the shooter's strength: what's sent to the
/// other player so they can replay it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Volley {
    pub weapon: Weapon,
    pub damage: f32,
    pub pellets: u32,
    pub blast_radius: f32,
    pub mods: Mods,
    /// Seeking bolts (Arcane Missiles): they curve towards targets.
    pub homing: bool,
}

/// What happened this frame.
#[derive(Default)]
pub struct Events {
    /// Drones you destroyed.
    pub kills: u32,
    /// Drones the other player destroyed (counted by the host).
    pub remote_kills: u32,
    /// Where drones went down.
    pub destroyed: Vec<Vec3>,
    /// Total shove from blasts on the player, m/s.
    pub push: Vec3,
    /// Health the player lost to the other player's shots.
    pub player_damage: f32,
    /// Your shots that struck a drone (for the hit marker).
    pub hits: u32,
    /// Damage you dealt to drones, and the melee part of it (lifesteal).
    pub dealt: f32,
    pub melee_dealt: f32,
    /// Your melee hits and the status effects you started (trials).
    pub melee_hits: u32,
    pub effects: u32,
}

impl Events {
    fn credit(&mut self, at: Vec3, remote: bool) {
        if remote {
            self.remote_kills += 1;
        } else {
            self.kills += 1;
        }
        self.destroyed.push(at);
    }
}

/// The effect a weapon's hit has, with its upgrades.
fn effect_of(weapon: Weapon, mods: Mods, dir: Vec3) -> Effect {
    let p = mods.power();
    let flat = Vec3::new(dir.x, 0.0, dir.z).normalize_or_zero();
    match weapon {
        Weapon::Blade => Effect { bleed: if mods.has(Mods::BLEED) { 1.2 * p } else { 0.0 }, knock: flat * 2.0, ..Default::default() },
        Weapon::Hammer => Effect {
            stun: if mods.has(Mods::CONCUSSION) { 2.5 } else { 1.0 } * p,
            knock: flat * 14.0 + Vec3::Y * 4.0,
            ..Default::default()
        },
        Weapon::Scythe => Effect { bleed: 1.6 * p, knock: flat * 4.0, ..Default::default() },
        Weapon::Blaster => {
            let ion = mods.has(Mods::ION);
            Effect {
                shock_every: if ion { 3 } else { 4 },
                shock_bonus: if ion { 4.0 } else { 2.0 } * p,
                chain: ion,
                ..Default::default()
            }
        }
        Weapon::Scatter => {
            let slug = mods.has(Mods::SLUG);
            Effect {
                knock: dir * if slug { 3.0 } else { 1.5 },
                stun: if slug { 0.3 * p } else { 0.0 },
                ..Default::default()
            }
        }
        Weapon::Rockets => {
            let burn = if mods.has(Mods::NAPALM) { (2.4 * p, 6.0) } else { (0.8 * p, 3.0) };
            Effect { burn, ..Default::default() }
        }
        Weapon::Railgun => {
            let over = mods.has(Mods::OVERCHARGE);
            Effect {
                crit: if over { 3.0 } else { 2.0 },
                shock_every: if over { 1 } else { 0 },
                shock_bonus: 1.5 * p,
                ..Default::default()
            }
        }
    }
}

pub struct Weapons {
    pub current: Weapon,
    /// The player's feet while a fight is on: the other player's shots
    /// can hurt them.
    pub victim: Option<Vec3>,
    cooldown: f32,
    shots: Vec<Shot>,
    blade_waves: Vec<BladeWave>,
    rift_gates: [Option<crate::net::GatePair>; 2],
    blasts: Vec<Blast>,
    beams: Vec<Beam>,
    slashes: Vec<Slash>,
    rng: u32,
    /// Explosions since last taken (position, radius), for sounds.
    pub booms: Vec<(Vec3, f32)>,
    /// Your blasts big enough to dent the ground, since last taken
    /// (position, blast radius).
    pub craters: Vec<(Vec3, f32)>,
    /// Structures that stop shots (set each frame), and the hits on them
    /// since last taken: (id, damage, where).
    pub blockers: Blockers,
    pub struct_hits: Vec<(u32, f32, Vec3)>,
    /// Melee swings that connected since last taken (where, which weapon).
    pub impacts: Vec<(Vec3, Weapon)>,
    pub kills: u32,
    bolt_handles: Vec<usize>,
    rocket_handles: Vec<usize>,
    blast_handles: Vec<usize>,
    beam_handles: Vec<usize>,
    slash_handles: Vec<usize>,
    rift_handles: Vec<[usize; 2]>,
    rift_surface_handles: Vec<[usize; 2]>,
}

fn glow(color: Vec4) -> MaterialData {
    MaterialData {
        base_color: color,
        ground_occlusion: 0.0,
        ..Default::default()
    }
}

impl Weapons {
    pub fn new(renderer: &mut Renderer) -> Self {
        let mut bolt = MeshData::default();
        bolt.push_box(Vec3::ZERO, Vec3::new(0.07, 0.07, 0.9), Vec4::ONE, BoxUv::PerFace);
        let bolt_material = glow(Vec4::new(0.5, 2.4, 2.8, 1.0));
        let mut rocket = MeshData::default();
        rocket.push_box(Vec3::ZERO, Vec3::new(0.18, 0.18, 0.6), Vec4::ONE, BoxUv::PerFace);
        let rocket_material = glow(Vec4::new(2.6, 1.2, 0.3, 1.0));
        let blast = sphere_mesh(1.0, 8, 14);
        let blast_material = glow(Vec4::new(3.0, 1.5, 0.4, 1.0));
        let mut beam = MeshData::default();
        beam.push_box(Vec3::new(0.0, 0.0, 0.5), Vec3::new(0.12, 0.12, 1.0), Vec4::ONE, BoxUv::PerFace);
        let beam_material = glow(Vec4::new(2.2, 0.8, 3.0, 1.0));
        // A slash segment: a thin blade of light, 1 m long along +X.
        let mut slash = MeshData::default();
        slash.push_box(Vec3::ZERO, Vec3::new(1.0, 0.05, 0.02), Vec4::ONE, BoxUv::PerFace);
        let slash_material = glow(Vec4::new(2.6, 2.8, 3.2, 1.0));
        let mut rift = MeshData::default();
        let ring_points: Vec<Vec3> = (0..32)
            .map(|i| {
                let a = i as f32 / 32.0 * std::f32::consts::TAU;
                Vec3::new(a.cos() * 1.65, a.sin() * 1.65, 0.0)
            })
            .collect();
        for i in 0..ring_points.len() {
            let a = ring_points[i];
            let b = ring_points[(i + 1) % ring_points.len()];
            let delta = b - a;
            rift.push_cylinder(a, delta.normalize(), 0.09, delta.length(), 8, Vec4::ONE, 1.0);
        }
        let mut aperture = MeshData::default();
        let center = Vec3::new(0.0, 0.0, -0.025);
        for i in 0..32 {
            let a0 = i as f32 / 32.0 * std::f32::consts::TAU;
            let a1 = (i + 1) as f32 / 32.0 * std::f32::consts::TAU;
            let p0 = Vec3::new(a0.cos() * 1.55, a0.sin() * 1.55, -0.025);
            let p1 = Vec3::new(a1.cos() * 1.55, a1.sin() * 1.55, -0.025);
            let uv = |a: f32| Vec2::new(0.5 + 0.5 * a.cos(), 0.5 - 0.5 * a.sin());
            aperture.push_tri([center, p0, p1], [Vec2::splat(0.5), uv(a0), uv(a1)], Vec4::ONE, Vec3::Z);
        }
        let rift_surface_handles: Vec<[usize; 2]> = (0..2)
            .map(|owner| [
                renderer.add_portal_surface(&aperture, owner * 2),
                renderer.add_portal_surface(&aperture, owner * 2 + 1),
            ])
            .collect();
        let rift_material = glow(Vec4::new(2.6, 0.55, 3.5, 1.0));

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
            current: Weapon::Blade,
            victim: None,
            cooldown: 0.0,
            shots: Vec::new(),
            blade_waves: Vec::new(),
            rift_gates: [None, None],
            blasts: Vec::new(),
            beams: Vec::new(),
            slashes: Vec::new(),
            rng: 0x9e37_79b9,
            booms: Vec::new(),
            craters: Vec::new(),
            impacts: Vec::new(),
            blockers: Blockers::default(),
            struct_hits: Vec::new(),
            kills: 0,
            bolt_handles: pool(MAX_SHOTS, &bolt, &bolt_material),
            rocket_handles: pool(MAX_ROCKETS, &rocket, &rocket_material),
            blast_handles: pool(MAX_BLASTS, &blast, &blast_material),
            beam_handles: pool(MAX_BEAMS, &beam, &beam_material),
            slash_handles: pool(MAX_SLASHES * SLASH_SEGMENTS, &slash, &slash_material),
            rift_handles: (0..2)
                .map(|_| [renderer.add_portal_frame(&rift, &rift_material), renderer.add_portal_frame(&rift, &rift_material)])
                .collect(),
            rift_surface_handles,
        }
    }

    #[cfg(test)]
    fn bare() -> Self {
        Self {
            current: Weapon::Blaster,
            victim: None,
            cooldown: 0.0,
            shots: Vec::new(),
            blade_waves: Vec::new(),
            rift_gates: [None, None],
            blasts: Vec::new(),
            beams: Vec::new(),
            slashes: Vec::new(),
            rng: 1,
            booms: Vec::new(),
            craters: Vec::new(),
            impacts: Vec::new(),
            blockers: Blockers::default(),
            struct_hits: Vec::new(),
            kills: 0,
            bolt_handles: Vec::new(),
            rocket_handles: Vec::new(),
            blast_handles: Vec::new(),
            beam_handles: Vec::new(),
            slash_handles: Vec::new(),
            rift_handles: Vec::new(),
            rift_surface_handles: Vec::new(),
        }
    }

    fn random(&mut self) -> f32 {
        // xorshift32
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng as f32 / u32::MAX as f32
    }

    /// Picks a weapon if it's unlocked.
    pub fn select(&mut self, weapon: Weapon, moves: &Moveset) -> bool {
        if moves.has_weapon(weapon) && self.current != weapon {
            self.current = weapon;
            self.cooldown = self.cooldown.min(0.2);
            return true;
        }
        false
    }

    /// Makes sure the weapon in hand is one you have (after a reset or a
    /// new unlock): the best gun, else the best melee weapon.
    pub fn ensure_owned(&mut self, moves: &Moveset) {
        if moves.has_weapon(self.current) {
            return;
        }
        if let Some(w) = Weapon::ALL.into_iter().rev().find(|w| moves.has_weapon(*w)) {
            self.current = w;
        }
    }

    /// Fraction of the reload done (1 = ready).
    pub fn ready_fraction(&self, moves: &Moveset) -> f32 {
        1.0 - self.cooldown / (self.current.delay() / self.current.rate(moves))
    }

    /// Ticks the reload; uses real time so slowing the world doesn't jam
    /// your gun.
    pub fn reload(&mut self, dt: f32) {
        self.cooldown = (self.cooldown - dt).max(0.0);
    }

    pub fn set_rift_gate(&mut self, owner: usize, pair: Option<crate::net::GatePair>) {
        if let Some(gate) = self.rift_gates.get_mut(owner) {
            *gate = pair;
        }
    }

    /// Fires the current weapon from each `(origin, direction)` (you, plus
    /// clones with Overcharge) if it's ready. Returns what happened and
    /// the volley fired (to send to the other player). Drones only take
    /// damage with `authority` (offline, or the host).
    pub fn fire(
        &mut self,
        shooters: &[(Vec3, Vec3)],
        moves: &Moveset,
        authority: bool,
        drones: &mut Drones,
        colliders: &[Aabb],
        terrain: &Terrain,
    ) -> (Events, Option<Volley>) {
        if self.cooldown > 0.0 || !moves.has_weapon(self.current) {
            return (Events::default(), None);
        }
        self.cooldown = self.current.delay() / self.current.rate(moves);
        let damage = if self.current.is_melee() { moves.weapon_damage * moves.melee_damage } else { moves.weapon_damage };
        let volley = Volley {
            weapon: self.current,
            damage,
            pellets: moves.pellets,
            blast_radius: moves.blast_radius,
            mods: Mods::from_moves(moves),
            homing: false,
        };
        let events = self.volley(&volley, shooters, false, authority, drones, colliders, terrain);
        (events, Some(volley))
    }

    /// One drone struck, with the weapon's effect; counts what it did.
    #[allow(clippy::too_many_arguments)]
    fn strike(drones: &mut Drones, i: usize, amount: f32, effect: &Effect, remote: bool, melee: bool, events: &mut Events) {
        let r = drones.hit(i, amount, effect, remote);
        if !remote {
            events.dealt += r.dealt;
            events.effects += r.effects;
            if melee {
                events.melee_dealt += r.dealt;
                events.melee_hits += 1;
            }
        }
        for at in [r.killed, r.chain_kill].into_iter().flatten() {
            events.credit(at, remote);
        }
    }

    /// Spawns a volley's shots: yours, or (`remote`) the other player's.
    #[allow(clippy::too_many_arguments)]
    pub fn volley(
        &mut self,
        v: &Volley,
        shooters: &[(Vec3, Vec3)],
        remote: bool,
        authority: bool,
        drones: &mut Drones,
        colliders: &[Aabb],
        terrain: &Terrain,
    ) -> Events {
        let mut events = Events::default();
        for &(origin, dir) in shooters {
            let dir = dir.normalize_or(Vec3::X);
            match v.weapon {
                Weapon::Blade if v.mods.has(Mods::BLADE_CONE) => {
                    let distance = v.blast_radius.clamp(4.0, 40.0);
                    if self.slashes.len() >= MAX_SLASHES {
                        self.slashes.remove(0);
                    }
                    self.slashes.push(Slash { origin, dir, reach: 3.0, arc: 0.9, age: 0.0, wave: true, duration: distance / 40.0 });
                    if self.blade_waves.len() >= 8 {
                        self.blade_waves.remove(0);
                    }
                    self.blade_waves.push(BladeWave { pos: origin, dir, damage: v.damage * 1.8, mods: v.mods, remote, life: distance / 40.0, hit: Vec::new() });
                }
                Weapon::Blade | Weapon::Hammer | Weapon::Scythe => self.swing(v, origin, dir, remote, authority, drones, &mut events),
                Weapon::Blaster => self.spawn(origin, dir * 90.0, Kind::Bolt, v, v.damage, 0.0, remote),
                Weapon::Scatter => {
                    let (a_axis, b_axis) = dir.any_orthonormal_pair();
                    let damage = if v.mods.has(Mods::SLUG) { v.damage * 1.5 } else { v.damage };
                    // Seeking bolts fan out wide and slow, then curve in.
                    let (spread, speed) = if v.homing { (0.45, HOMING_SPEED) } else { (0.08, 75.0) };
                    for _ in 0..v.pellets.min(32) {
                        let a = self.random() * std::f32::consts::TAU;
                        let r = self.random().sqrt() * spread;
                        let d = (dir + (a_axis * a.cos() + b_axis * a.sin()) * r).normalize();
                        self.spawn(origin, d * speed, Kind::Bolt, v, damage, 0.0, remote);
                    }
                }
                Weapon::Rockets => self.spawn(origin, dir * 32.0, Kind::Rocket, v, 6.0 * v.damage, v.blast_radius, remote),
                Weapon::Railgun => {
                    let mut end = raycast(origin, dir, RAIL_RANGE, colliders, terrain)
                        .map_or(RAIL_RANGE, |p| p.distance(origin));
                    // A barrier or dome stops the beam (and takes the hit).
                    if let Some((t, id)) = self.blockers.hit(origin, dir, end) {
                        end = t;
                        self.struct_hits.push((id, 8.0 * v.damage * SHOT_SCALE, origin + dir * t));
                    }
                    // The other player's beam through you.
                    if remote
                        && let Some(feet) = self.victim
                        && segment_hits_body(origin, dir, end, feet).is_some()
                    {
                        events.player_damage += 8.0 * v.damage * PVP_SCALE;
                    }
                    // Pierces every drone along the line.
                    if authority {
                        let effect = effect_of(Weapon::Railgun, v.mods, dir);
                        for (i, _) in drones.ray_hits(origin, dir, end) {
                            events.hits += u32::from(!remote);
                            Self::strike(drones, i, 8.0 * v.damage, &effect, remote, false, &mut events);
                        }
                    }
                    if self.beams.len() >= MAX_BEAMS {
                        self.beams.remove(0);
                    }
                    self.beams.push(Beam { from: origin, to: origin + dir * end, age: 0.0 });
                }
            }
        }
        self.finish(events)
    }

    /// A melee swing: everything in the arc in front of `origin` is hit.
    #[allow(clippy::too_many_arguments)]
    fn swing(&mut self, v: &Volley, origin: Vec3, dir: Vec3, remote: bool, authority: bool, drones: &mut Drones, events: &mut Events) {
        let (reach, arc, base) = v.weapon.swing();
        let damage = base * v.damage;
        let effect = effect_of(v.weapon, v.mods, dir);
        let in_arc = |p: Vec3| {
            let to = p - origin;
            let d = to.length();
            d < reach + DRONE_RADIUS && (d < 0.8 || to.normalize().dot(dir).acos() <= arc)
        };
        let mut landed = None;
        let targets: Vec<usize> = drones.within(origin, reach).into_iter().map(|(i, _)| i).collect();
        for i in targets {
            let Some(pos) = drones.position(i) else { continue };
            if !in_arc(pos) {
                continue;
            }
            landed = Some(pos);
            if !remote {
                events.hits += 1;
            }
            if authority {
                Self::strike(drones, i, damage, &effect, remote, true, events);
            }
        }
        // The other player's swing at you.
        if remote
            && let Some(feet) = self.victim
            && in_arc(feet + Vec3::Y * 0.9)
        {
            events.player_damage += damage * PVP_SCALE;
            landed = Some(feet + Vec3::Y);
        }
        // Concussion: the hammer's blow ripples out around where it lands.
        if v.weapon == Weapon::Hammer && v.mods.has(Mods::CONCUSSION) {
            let at = landed.unwrap_or(origin + dir * reach * 0.7);
            let ripple = Effect { stun: 1.2 * v.mods.power(), ..Default::default() };
            self.explode(at, 3.5, damage * 0.4, remote, authority, drones, at + Vec3::Y * 1000.0, &ripple, events);
        }
        if let Some(at) = landed
            && !remote
        {
            self.impacts.push((at, v.weapon));
        }
        if self.slashes.len() >= MAX_SLASHES {
            self.slashes.remove(0);
        }
        self.slashes.push(Slash { origin, dir, reach, arc: arc.min(1.4), age: 0.0, wave: false, duration: SLASH_TIME });
    }

    /// Bookkeeping after hits: count your kills and blow up the drones.
    fn finish(&mut self, events: Events) -> Events {
        for &at in &events.destroyed {
            self.add_blast(at, 1.6);
        }
        self.kills += events.kills;
        events
    }

    /// A beam of light between two points (Chain Lightning).
    pub fn add_beam(&mut self, from: Vec3, to: Vec3) {
        if self.beams.len() >= MAX_BEAMS {
            self.beams.remove(0);
        }
        self.beams.push(Beam { from, to, age: 0.0 });
    }

    /// A burst of light with no blast behind it (skills like Repulse).
    pub fn add_flash(&mut self, pos: Vec3, radius: f32) {
        if self.blasts.len() >= MAX_BLASTS {
            self.blasts.remove(0);
        }
        self.blasts.push(Blast { pos, radius, age: 0.0 });
    }

    /// Shows an explosion (also used for drones destroyed on the host).
    pub fn add_blast(&mut self, pos: Vec3, radius: f32) {
        if self.blasts.len() >= MAX_BLASTS {
            self.blasts.remove(0);
        }
        self.blasts.push(Blast { pos, radius, age: 0.0 });
        if self.booms.len() < 16 {
            self.booms.push((pos, radius));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn(&mut self, pos: Vec3, vel: Vec3, kind: Kind, v: &Volley, damage: f32, blast: f32, remote: bool) {
        let (limit, count) = match kind {
            Kind::Bolt => (MAX_SHOTS, self.shots.iter().filter(|s| s.kind == Kind::Bolt).count()),
            Kind::Rocket => (MAX_ROCKETS, self.shots.iter().filter(|s| s.kind == Kind::Rocket).count()),
        };
        if count >= limit
            && let Some(oldest) = self.shots.iter().position(|s| s.kind == kind)
        {
            self.shots.remove(oldest);
        }
        self.shots.push(Shot { pos, vel, kind, weapon: v.weapon, mods: v.mods, damage, blast, life: SHOT_LIFE, remote, homing: v.homing });
    }

    /// An explosion: damages drones (less towards the edge, with
    /// `authority`) and shoves the player at `player` (their middle) away.
    #[allow(clippy::too_many_arguments)]
    fn explode(
        &mut self,
        pos: Vec3,
        radius: f32,
        damage: f32,
        remote: bool,
        authority: bool,
        drones: &mut Drones,
        player: Vec3,
        effect: &Effect,
        events: &mut Events,
    ) {
        // A powerful blast of your own leaves a crater (the other player's
        // send theirs).
        if !remote && radius >= CRATER_BLAST && self.craters.len() < 8 {
            self.craters.push((pos, radius));
        }
        if authority {
            for (i, dist) in drones.within(pos, radius) {
                events.hits += u32::from(!remote);
                let falloff = 1.0 - 0.6 * (dist / radius).min(1.0);
                let mut e = *effect;
                if let Some(p) = drones.position(i) {
                    e.knock += (p - pos).normalize_or(Vec3::Y) * 6.0;
                }
                Self::strike(drones, i, damage * falloff, &e, remote, false, events);
            }
        }
        if remote
            && let Some(feet) = self.victim
        {
            let dist = body_distance(pos, feet).max(0.0);
            if dist < radius {
                events.player_damage += damage * PVP_SCALE * (1.0 - 0.6 * dist / radius);
            }
        }
        let away = player - pos;
        let dist = away.length();
        if dist < radius {
            let strength = BLAST_PUSH * (1.0 - dist / radius);
            events.push += away.normalize_or(Vec3::Y) * strength + Vec3::Y * strength * 0.35;
        }
        self.add_blast(pos, radius);
    }

    /// A ground slam's shockwave: drones only, no shove; they're stunned.
    /// An ability's blast with its own size, damage and effect: yours, or
    /// (`remote`) the other player's, which can hurt you.
    #[allow(clippy::too_many_arguments)]
    pub fn burst(&mut self, pos: Vec3, radius: f32, damage: f32, effect: &Effect, remote: bool, authority: bool, drones: &mut Drones) -> Events {
        let mut events = Events::default();
        self.explode(pos, radius, damage, remote, authority, drones, pos + Vec3::Y * 1000.0, effect, &mut events);
        self.finish(events)
    }

    pub fn shockwave(&mut self, pos: Vec3, damage: f32, remote: bool, authority: bool, drones: &mut Drones) -> Events {
        let mut events = Events::default();
        let stun = Effect { stun: 0.8, ..Default::default() };
        // A shockwave (slam, repulse) is a push of air: no crater of its own.
        let craters = self.craters.len();
        self.explode(pos, 8.0, 4.0 * damage, remote, authority, drones, pos + Vec3::Y * 1000.0, &stun, &mut events);
        self.craters.truncate(craters);
        self.finish(events)
    }

    /// Moves shots (in world time) and resolves hits. `player` is the
    /// player's middle, for blast shoves. Drones only take damage with
    /// `authority`; elsewhere shots just stop when they hit one.
    pub fn update(
        &mut self,
        dt: f32,
        authority: bool,
        drones: &mut Drones,
        colliders: &[Aabb],
        terrain: &Terrain,
        player: Vec3,
    ) -> Events {
        let mut events = Events::default();
        let gates = self.rift_gates;
        let mut wave_i = 0;
        while wave_i < self.blade_waves.len() {
            let wave = &mut self.blade_waves[wave_i];
            let start = wave.pos;
            let travel = wave.dir * (40.0 * dt);
            let length = travel.length();
            let effect = effect_of(Weapon::Blade, wave.mods, wave.dir);
            let stop_at = self.blockers.hit(start, wave.dir, length).map(|(t, id)| {
                self.struct_hits.push((id, wave.damage * SHOT_SCALE, start + wave.dir * t));
                t
            }).or_else(|| raycast(start, wave.dir, length, colliders, terrain).map(|p| p.distance(start)));
            let limit = stop_at.unwrap_or(length);
            let midpoint = start + wave.dir * (limit * 0.5);
            for (target, _) in drones.within(midpoint, limit * 0.5 + 2.2) {
                let Some(at) = drones.position(target) else { continue };
                let along = (at - start).dot(wave.dir).clamp(0.0, limit);
                if !wave.hit.contains(&target) && at.distance(start + wave.dir * along) <= DRONE_RADIUS + 1.35 {
                    events.hits += u32::from(!wave.remote);
                    if authority {
                        Self::strike(drones, target, wave.damage, &effect, wave.remote, false, &mut events);
                    }
                    wave.hit.push(target);
                }
            }
            let hit_player = wave.remote
                && self.victim.is_some_and(|feet| segment_hits_body(start, wave.dir, limit, feet).is_some());
            if hit_player {
                events.player_damage += wave.damage * PVP_SCALE;
            }
            wave.pos += travel;
            wave.life -= dt;
            if stop_at.is_some() || hit_player || wave.life <= 0.0 {
                self.blade_waves.remove(wave_i);
            } else {
                wave_i += 1;
            }
        }
        let mut i = 0;
        while i < self.shots.len() {
            let shot = &mut self.shots[i];
            shot.life -= dt;
            if let Some((pos, vel)) = gate_traverse(&gates, shot.pos, shot.vel, dt) {
                shot.pos = pos;
                shot.vel = vel;
                i += 1;
                continue;
            }
            // Seeking bolts curve towards the nearest target ahead: a drone,
            // or (the other player's) you.
            if shot.homing && dt > 0.0 {
                let speed = shot.vel.length();
                let ahead = shot.vel / speed.max(1e-3);
                let mut best: Option<(f32, Vec3)> = None;
                let mut consider = |p: Vec3| {
                    let to = p - shot.pos;
                    let d = to.length();
                    let facing = to.dot(ahead) / d.max(1e-3);
                    // Nearer and straighter ahead is better.
                    let score = d * (2.0 - facing);
                    if facing > 0.2 && best.is_none_or(|(s, _)| score < s) {
                        best = Some((score, p));
                    }
                };
                for (j, _) in drones.within(shot.pos, HOMING_RANGE) {
                    if let Some(p) = drones.position(j) {
                        consider(p);
                    }
                }
                if shot.remote
                    && let Some(feet) = self.victim
                    && feet.distance(shot.pos) < HOMING_RANGE
                {
                    consider(feet + Vec3::Y * 0.9);
                }
                if let Some((_, p)) = best {
                    let want = (p - shot.pos).normalize_or(ahead);
                    let turned = (ahead + (want - ahead) * (HOMING_TURN * dt).min(1.0)).normalize_or(ahead);
                    shot.vel = turned * speed;
                }
            }
            let travel = shot.vel * dt;
            let len = travel.length();
            let dir = travel.normalize_or(Vec3::X);
            let (pos, kind, weapon, mods, damage, blast, remote, expired) =
                (shot.pos, shot.kind, shot.weapon, shot.mods, shot.damage, shot.blast, shot.remote, shot.life <= 0.0);
            if len == 0.0 {
                // Frozen by Time Stop.
                i += 1;
                continue;
            }
            let effect = effect_of(weapon, mods, dir);
            // The other player's shots can hit you (during a fight).
            // A structure in the way stops the shot first.
            if let Some((t, id)) = self.blockers.hit(pos, dir, len) {
                let closer_body = remote && self.victim.and_then(|feet| segment_hits_body(pos, dir, len, feet)).is_some_and(|b| b < t);
                let closer_drone = drones.ray_hit(pos, dir, len).is_some_and(|d| d.1 < t);
                if !closer_body && !closer_drone {
                    let at = pos + dir * t;
                    self.shots.remove(i);
                    self.struct_hits.push((id, damage * SHOT_SCALE, at));
                    if kind == Kind::Rocket {
                        // Explode just in front of the surface.
                        self.explode(at - dir * 0.3, blast, damage, remote, authority, drones, player, &effect, &mut events);
                    }
                    continue;
                }
            }
            let body = if remote { self.victim.and_then(|feet| segment_hits_body(pos, dir, len, feet)) } else { None };
            let drone = drones.ray_hit(pos, dir, len);
            let nearest_other = drone.map(|d| d.1).into_iter().chain(raycast(pos, dir, len, colliders, terrain).map(|p| p.distance(pos))).fold(f32::MAX, f32::min);
            if let Some(t) = body.filter(|t| *t <= nearest_other) {
                self.shots.remove(i);
                match kind {
                    Kind::Bolt => events.player_damage += damage * PVP_SCALE,
                    Kind::Rocket => self.explode(pos + dir * t, blast, damage, remote, authority, drones, player, &effect, &mut events),
                }
                continue;
            }
            let wall = raycast(pos, dir, len, colliders, terrain).map(|p| p.distance(pos));
            let hit = match (drone, wall) {
                (Some((_, t)), Some(w)) if w < t => Some((None, w)),
                (Some((d, t)), _) => Some((Some(d), t)),
                (None, Some(w)) => Some((None, w)),
                (None, None) => None,
            };
            match hit {
                Some((target, t)) => {
                    let at = pos + dir * t;
                    self.shots.remove(i);
                    match kind {
                        Kind::Bolt => {
                            events.hits += u32::from(!remote && target.is_some());
                            if authority && let Some(d) = target {
                                Self::strike(drones, d, damage, &effect, remote, false, &mut events);
                            }
                        }
                        Kind::Rocket => self.explode(at, blast, damage, remote, authority, drones, player, &effect, &mut events),
                    }
                }
                None if expired => {
                    let shot = self.shots.remove(i);
                    if kind == Kind::Rocket {
                        self.explode(shot.pos, blast, damage, remote, authority, drones, player, &effect, &mut events);
                    }
                }
                None => {
                    self.shots[i].pos += travel;
                    i += 1;
                }
            }
        }
        for b in &mut self.blasts {
            b.age += dt;
        }
        self.blasts.retain(|b| b.age < BLAST_TIME);
        for b in &mut self.beams {
            b.age += dt.max(1.0 / 120.0); // beams fade even in stopped time
        }
        self.beams.retain(|b| b.age < BEAM_TIME);
        for s in &mut self.slashes {
            s.age += dt.max(1.0 / 120.0);
            if s.wave {
                s.origin += s.dir * (40.0 * dt);
            }
        }
        self.slashes.retain(|s| s.age < s.duration);
        self.finish(events)
    }
    pub fn draw(&self, renderer: &mut Renderer) {
        let mut bolts = self.shots.iter().filter(|s| s.kind == Kind::Bolt);
        for &h in &self.bolt_handles {
            match bolts.next() {
                Some(s) => renderer.set_transform(h, Self::aligned(s.pos, s.vel, 1.0), true),
                None => renderer.set_transform(h, Mat4::IDENTITY, false),
            }
        }
        let mut rockets = self.shots.iter().filter(|s| s.kind == Kind::Rocket);
        for &h in &self.rocket_handles {
            match rockets.next() {
                Some(s) => renderer.set_transform(h, Self::aligned(s.pos, s.vel, 1.0), true),
                None => renderer.set_transform(h, Mat4::IDENTITY, false),
            }
        }
        for (i, &h) in self.blast_handles.iter().enumerate() {
            match self.blasts.get(i) {
                Some(b) => {
                    let grow = (b.age / BLAST_TIME).sqrt();
                    let t = Mat4::from_scale_rotation_translation(Vec3::splat(b.radius * grow), Quat::IDENTITY, b.pos);
                    renderer.set_transform(h, t, true);
                }
                None => renderer.set_transform(h, Mat4::IDENTITY, false),
            }
        }
        for (i, &h) in self.beam_handles.iter().enumerate() {
            match self.beams.get(i) {
                Some(b) => {
                    let span = b.to - b.from;
                    let thin = 1.0 - b.age / BEAM_TIME;
                    let t = Mat4::from_scale_rotation_translation(
                        Vec3::new(thin, thin, span.length()),
                        Quat::from_rotation_arc(Vec3::Z, span.normalize_or(Vec3::Z)),
                        b.from,
                    );
                    renderer.set_transform(h, t, true);
                }
                None => renderer.set_transform(h, Mat4::IDENTITY, false),
            }
        }
        // Slashes: an arc of short glowing segments sweeping across.
        for (s_i, chunk) in self.slash_handles.chunks(SLASH_SEGMENTS).enumerate() {
            let Some(s) = self.slashes.get(s_i) else {
                for &h in chunk {
                    renderer.set_transform(h, Mat4::IDENTITY, false);
                }
                continue;
            };
            let t = s.age / s.duration;
            let side = Vec3::Y.cross(s.dir).normalize_or(Vec3::X);
            let r = s.reach * 0.75;
            let seg_len = 2.0 * r * (s.arc / SLASH_SEGMENTS as f32);
            for (k, &h) in chunk.iter().enumerate() {
                // The arc reveals itself from one side to the other.
                let f = k as f32 / (SLASH_SEGMENTS - 1) as f32;
                let shown = f <= t * 2.0;
                let a = (f * 2.0 - 1.0) * s.arc;
                let dir = (s.dir * a.cos() + side * a.sin()).normalize();
                let tangent = (side * a.cos() - s.dir * a.sin()).normalize();
                let pos = s.origin + dir * r - Vec3::Y * 0.15 * (f - 0.5);
                let thin = 1.0 - t;
                let m = Mat4::from_scale_rotation_translation(
                    Vec3::new(seg_len * 1.15, thin * 1.5, thin),
                    Quat::from_rotation_arc(Vec3::X, tangent),
                    pos,
                );
                renderer.set_transform(h, m, shown);
            }
        }
        for (owner, handles) in self.rift_handles.iter().enumerate() {
            if let Some(pair) = self.rift_gates[owner] {
                for (handle, point) in handles.iter().zip([pair.a, pair.b]) {
                    let transform = Mat4::from_rotation_translation(Quat::from_rotation_y(point.yaw - std::f32::consts::FRAC_PI_2), point.pos);
                    renderer.set_transform(*handle, transform, true);
                }
                for (handle, point) in self.rift_surface_handles[owner].iter().zip([pair.a, pair.b]) {
                    let transform = Mat4::from_rotation_translation(Quat::from_rotation_y(point.yaw - std::f32::consts::FRAC_PI_2), point.pos);
                    renderer.set_transform(*handle, transform, true);
                }
            } else {
                for &handle in handles {
                    renderer.set_transform(handle, Mat4::IDENTITY, false);
                }
                for &handle in &self.rift_surface_handles[owner] {
                    renderer.set_transform(handle, Mat4::IDENTITY, false);
                }
            }
        }
    }

    /// A transform placing a +Z-pointing mesh at `pos` along `dir`.
    fn aligned(pos: Vec3, dir: Vec3, scale: f32) -> Mat4 {
        Mat4::from_scale_rotation_translation(
            Vec3::splat(scale),
            Quat::from_rotation_arc(Vec3::Z, dir.normalize_or(Vec3::Z)),
            pos,
        )
    }
}

/// If a shot segment crosses a gate aperture, preserve its remainder of
/// this frame's travel and rotate its velocity to face out of the linked end.
fn gate_traverse(
    gates: &[Option<crate::net::GatePair>; 2],
    pos: Vec3,
    velocity: Vec3,
    dt: f32,
) -> Option<(Vec3, Vec3)> {
    let end = pos + velocity * dt;
    for pair in gates.iter().flatten() {
        for (source, destination) in [(pair.a, pair.b), (pair.b, pair.a)] {
            let normal = Vec3::new(source.yaw.cos(), 0.0, source.yaw.sin());
            let right = Vec3::new(-normal.z, 0.0, normal.x);
            let d0 = (pos - source.pos).dot(normal);
            let d1 = (end - source.pos).dot(normal);
            if d0 * d1 > 0.0 || (d0 - d1).abs() < 1e-5 { continue; }
            let t = (d0 / (d0 - d1)).clamp(0.0, 1.0);
            let crossing = pos.lerp(end, t);
            let offset = crossing - source.pos;
            let lateral = offset.dot(right);
            if lateral.abs() > 1.65 || offset.y.abs() > 1.65 { continue; }
            let side = if d0 >= 0.0 { 1.0 } else { -1.0 };
            let rotation = Quat::from_rotation_y(destination.yaw - source.yaw + std::f32::consts::PI);
            let exit_normal = Vec3::new(destination.yaw.cos(), 0.0, destination.yaw.sin());
            let exit_right = Vec3::new(-exit_normal.z, 0.0, exit_normal.x);
            let exit = destination.pos + exit_normal * side * 1.2 + exit_right * lateral + Vec3::Y * offset.y;
            let remainder = rotation * (end - crossing);
            return Some((exit + remainder, rotation * velocity));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::items::{BLASTER, RAILGUN, ROCKETS, SCATTER};

    /// The guns an old Arsenal level gave: the first `level` of them.
    fn arsenal(level: u32) -> Moveset {
        let guns: Vec<usize> = [BLASTER, SCATTER, ROCKETS, RAILGUN].into_iter().take(level as usize).collect();
        Moveset::build(&[], &guns, 0)
    }

    fn drones_at(points: &[Vec3]) -> Drones {
        Drones::test_at(points)
    }

    #[test]
    fn the_other_players_ability_blasts_hurt_you_only_inside_them() {
        let mut drones = drones_at(&[]);
        let mut w = Weapons::bare();
        w.victim = Some(Vec3::new(3.0, 0.0, 0.0));
        let calm = Effect::default();
        let hit = w.burst(Vec3::new(0.0, 1.0, 0.0), 5.0, 5.0, &calm, true, false, &mut drones);
        assert!(hit.player_damage > 5.0 * PVP_SCALE * 0.4, "a Whirlwind 3 m away: {}", hit.player_damage);
        let far = w.burst(Vec3::new(-20.0, 1.0, 0.0), 5.0, 5.0, &calm, true, false, &mut drones);
        assert_eq!(far.player_damage, 0.0);
        let mine = w.burst(Vec3::new(0.0, 1.0, 0.0), 5.0, 5.0, &calm, false, false, &mut drones);
        assert_eq!(mine.player_damage, 0.0, "your own blast doesn't hurt you");
    }

    #[test]
    fn seeking_bolts_curve_onto_a_drone_off_to_the_side() {
        let moves = Moveset::default();
        let flat = Terrain::flat(100.0);
        let missiles = |homing| Volley { weapon: Weapon::Scatter, damage: 3.0, pellets: 6, blast_radius: 0.0, mods: Mods::default(), homing };
        let shooters = [(Vec3::new(0.0, 2.0, 0.0), Vec3::X)];
        // Plain bolts fly straight past it...
        let mut drones = drones_at(&[Vec3::new(20.0, 2.0, 9.0)]);
        let mut w = Weapons::bare();
        w.volley(&missiles(false), &shooters, false, true, &mut drones, &[], &flat);
        assert_eq!(run(&mut w, &moves, &mut drones, 120, Vec3::new(0.0, -50.0, 0.0)).kills, 0);
        // ...seeking ones turn and hit.
        let mut drones = drones_at(&[Vec3::new(20.0, 2.0, 9.0)]);
        let mut w = Weapons::bare();
        w.volley(&missiles(true), &shooters, false, true, &mut drones, &[], &flat);
        assert_eq!(run(&mut w, &moves, &mut drones, 120, Vec3::new(0.0, -50.0, 0.0)).kills, 1);
    }

    fn run(w: &mut Weapons, _moves: &Moveset, drones: &mut Drones, frames: usize, player: Vec3) -> Events {
        let flat = Terrain::flat(100.0);
        let mut all = Events::default();
        for _ in 0..frames {
            let e = w.update(1.0 / 60.0, true, drones, &[], &flat, player);
            all.kills += e.kills;
            all.push += e.push;
            all.player_damage += e.player_damage;
        }
        all
    }

    #[test]
    fn blaster_bolts_fly_and_destroy_drones() {
        let moves = arsenal(1);
        let mut drones = drones_at(&[Vec3::new(30.0, 2.0, 0.0)]);
        let mut w = Weapons::bare();
        let flat = Terrain::flat(100.0);
        let origin = [(Vec3::new(0.0, 2.0, 0.0), Vec3::X)];
        let mut kills = 0;
        for _ in 0..6 {
            w.fire(&origin, &moves, true, &mut drones, &[], &flat);
            w.reload(1.0);
            kills += run(&mut w, &moves, &mut drones, 30, Vec3::new(0.0, -50.0, 0.0)).kills;
        }
        assert_eq!(kills, 1, "six bolts destroy one drone");
        assert_eq!(drones.alive_count(), 0);
    }

    #[test]
    fn melee_hits_the_arc_in_front_and_effects_apply() {
        let mut moves = Moveset::build(&[], &[crate::items::BLADE, crate::items::WARHAMMER, crate::items::KEEN_EDGE], 0);
        moves.blade_cone = false; // the game only sends the blade wave on a charged release
        let flat = Terrain::flat(100.0);
        // One drone in front, in reach; one behind; one too far.
        let mut drones = drones_at(&[Vec3::new(2.0, 1.3, 0.0), Vec3::new(-2.0, 1.3, 0.0), Vec3::new(9.0, 1.3, 0.0)]);
        let mut w = Weapons::bare();
        assert!(w.select(Weapon::Blade, &moves));
        let (e, volley) = w.fire(&[(Vec3::new(0.0, 1.3, 0.0), Vec3::X)], &moves, true, &mut drones, &[], &flat);
        assert_eq!(volley.map(|v| v.weapon), Some(Weapon::Blade));
        assert_eq!((e.hits, e.melee_hits, e.effects), (1, 1, 1), "one drone hit, and it bleeds");
        assert!(e.melee_dealt >= 3.0);
        // The hammer finishes it and stuns.
        w.select(Weapon::Hammer, &moves);
        w.reload(5.0);
        let (e, _) = w.fire(&[(Vec3::new(0.0, 1.3, 0.0), Vec3::X)], &moves, true, &mut drones, &[], &flat);
        assert_eq!(e.kills, 1);
        assert_eq!(drones.alive_count(), 2);
        // Guns aren't unlocked by melee alone.
        assert!(!w.select(Weapon::Blaster, &moves));
    }

    #[test]
    fn rockets_burn_and_bleeding_finishes_drones_off() {
        let flat = Terrain::flat(100.0);
        let mut drones = drones_at(&[Vec3::new(5.0, 2.0, 0.0)]);
        let burn = Effect { burn: (2.0, 6.0), ..Default::default() };
        let r = drones.hit(0, 1.0, &burn, false);
        assert_eq!((r.effects, r.killed), (1, None));
        // Five health left, 2 per second: gone within 3 s, credited to you.
        let mut renderer_free_kills = 0;
        for _ in 0..200 {
            renderer_free_kills += drones.tick_effects_for_test(1.0 / 60.0, &flat).len();
        }
        assert_eq!(renderer_free_kills, 1);
        assert_eq!(drones.alive_count(), 0);
    }

    #[test]
    fn locked_weapons_dont_fire() {
        let mut w = Weapons::bare();
        assert!(!w.select(Weapon::Railgun, &arsenal(2)));
        assert!(w.select(Weapon::Scatter, &arsenal(2)));
        let flat = Terrain::flat(100.0);
        let mut drones = drones_at(&[]);
        w.fire(&[(Vec3::ZERO, Vec3::X)], &Moveset::default(), true, &mut drones, &[], &flat);
        assert!(w.shots.is_empty(), "no Arsenal levels: nothing fires");
    }

    #[test]
    fn railgun_pierces_and_rockets_shove_you() {
        let moves = arsenal(4);
        let flat = Terrain::flat(100.0);
        let mut drones = drones_at(&[Vec3::new(10.0, 2.0, 0.0), Vec3::new(20.0, 2.0, 0.0)]);
        let mut w = Weapons::bare();
        w.select(Weapon::Railgun, &moves);
        let (e, volley) = w.fire(&[(Vec3::new(0.0, 2.0, 0.0), Vec3::X)], &moves, true, &mut drones, &[], &flat);
        assert_eq!(e.kills, 2, "one beam, two drones");
        assert_eq!(volley.map(|v| v.weapon), Some(Weapon::Railgun));

        // A rocket fired at the ground beside you pushes you up and away.
        w.select(Weapon::Rockets, &moves);
        w.reload(5.0);
        let you = Vec3::new(0.0, 0.9, 0.0);
        let mut no_drones = drones_at(&[]);
        w.fire(&[(Vec3::new(0.0, 1.5, 0.0), Vec3::new(1.0, -1.5, 0.0))], &moves, true, &mut no_drones, &[], &flat);
        let e = run(&mut w, &moves, &mut no_drones, 30, you);
        assert!(e.push.y > 5.0 && e.push.x < 0.0, "rocket jump push: {:?}", e.push);
    }

    #[test]
    fn only_the_authority_damages_drones_and_credits_the_shooter() {
        let moves = arsenal(4);
        let flat = Terrain::flat(100.0);
        let shot = [(Vec3::new(0.0, 2.0, 0.0), Vec3::X)];
        let volley = Volley { weapon: Weapon::Railgun, damage: 1.0, pellets: 8, blast_radius: 4.5, mods: Mods::default(), homing: false };

        // The joiner's own copy: the beam shows, but no damage.
        let mut drones = drones_at(&[Vec3::new(10.0, 2.0, 0.0)]);
        let mut w = Weapons::bare();
        w.select(Weapon::Railgun, &moves);
        let (e, _) = w.fire(&shot, &moves, false, &mut drones, &[], &flat);
        assert_eq!((e.kills, drones.alive_count()), (0, 1));

        // The host replaying the joiner's shot: damage, credited to them.
        let e = w.volley(&volley, &shot, true, true, &mut drones, &[], &flat);
        assert_eq!((e.kills, e.remote_kills, w.kills), (0, 1, 0));
    }

    #[test]
    fn the_other_players_shots_hurt_you_but_yours_dont() {
        let moves = arsenal(4);
        let flat = Terrain::flat(100.0);
        let mut drones = drones_at(&[]);
        let you = Vec3::new(20.0, 0.0, 0.0);
        let aim = [(Vec3::new(0.0, 1.0, 0.0), Vec3::X)];
        let bolt = Volley { weapon: Weapon::Blaster, damage: 1.0, pellets: 8, blast_radius: 4.5, mods: Mods::default(), homing: false };

        // Your own bolt flies through you.
        let mut w = Weapons::bare();
        w.victim = Some(you);
        w.volley(&bolt, &aim, false, true, &mut drones, &[], &flat);
        let e = run(&mut w, &moves, &mut drones, 30, you + Vec3::Y);
        assert_eq!(e.player_damage, 0.0);

        // Theirs hits.
        w.volley(&bolt, &aim, true, true, &mut drones, &[], &flat);
        let e = run(&mut w, &moves, &mut drones, 30, you + Vec3::Y);
        assert_eq!(e.player_damage, PVP_SCALE);

        // Their railgun hits at once; nobody is hurt outside a fight.
        let rail = Volley { weapon: Weapon::Railgun, ..bolt };
        assert_eq!(w.volley(&rail, &aim, true, true, &mut drones, &[], &flat).player_damage, 8.0 * PVP_SCALE);
        w.victim = None;
        assert_eq!(w.volley(&rail, &aim, true, true, &mut drones, &[], &flat).player_damage, 0.0);
    }
}
