//! Weapons (the Arsenal tree): Blaster, Scatter Gun, Rocket Launcher and
//! Railgun. Shots fly in world time, so Bullet Time slows them and Time
//! Stop freezes them in the air. Rocket blasts shove you as well as the
//! drones: aim at your feet for a rocket jump.

use glam::{Mat4, Quat, Vec3, Vec4};

use crate::abilities::Moveset;
use crate::arena::{PVP_SCALE, body_distance, segment_hits_body};
use crate::collision::Aabb;
use crate::drones::{Drones, sphere_mesh};
use crate::gadgets::raycast;
use crate::mesh::{BoxUv, MeshData};
use crate::model::MaterialData;
use crate::renderer::Renderer;
use crate::terrain::Terrain;

const MAX_SHOTS: usize = 96;
const MAX_ROCKETS: usize = 16;
const MAX_BLASTS: usize = 12;
const MAX_BEAMS: usize = 6;
const SHOT_LIFE: f32 = 2.5;
const BLAST_TIME: f32 = 0.3;
const BEAM_TIME: f32 = 0.2;
const RAIL_RANGE: f32 = 250.0;
/// How hard a blast at point-blank range shoves you, m/s.
const BLAST_PUSH: f32 = 24.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Weapon {
    Blaster,
    Scatter,
    Rockets,
    Railgun,
}

impl Weapon {
    pub const ALL: [Weapon; 4] = [Weapon::Blaster, Weapon::Scatter, Weapon::Rockets, Weapon::Railgun];

    pub fn name(self) -> &'static str {
        match self {
            Weapon::Blaster => "Blaster",
            Weapon::Scatter => "Scatter Gun",
            Weapon::Rockets => "Rocket Launcher",
            Weapon::Railgun => "Railgun",
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
            Weapon::Blaster => "1",
            Weapon::Scatter => "2",
            Weapon::Rockets => "3",
            Weapon::Railgun => "4",
        }
    }

    /// For the menu: (damage per shot, shots per second, what's special).
    pub fn stats(self, moves: &Moveset) -> (String, f32, &'static str) {
        let d = moves.weapon_damage;
        let rate = moves.fire_rate / self.delay();
        match self {
            Weapon::Blaster => (format!("{d:.1}"), rate, "Automatic"),
            Weapon::Scatter => {
                let pellets = moves.pellets;
                (format!("{pellets} x {d:.1}"), rate, "Spread, best up close")
            }
            Weapon::Rockets => (format!("{:.1} area", 6.0 * d), rate, "Blast shoves you: rocket jump"),
            Weapon::Railgun => (format!("{:.1}", 8.0 * d), rate, "Instant, pierces"),
        }
    }

    /// Seconds between shots at normal fire rate.
    fn delay(self) -> f32 {
        match self {
            Weapon::Blaster => 0.15,
            Weapon::Scatter => 0.9,
            Weapon::Rockets => 0.9,
            Weapon::Railgun => 1.1,
        }
    }

    /// Unlocked with this many Arsenal levels.
    pub fn unlocked_at(self) -> u32 {
        match self {
            Weapon::Blaster => 1,
            Weapon::Scatter => 2,
            Weapon::Rockets => 3,
            Weapon::Railgun => 4,
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
    damage: f32,
    /// Blast radius, for rockets.
    blast: f32,
    life: f32,
    /// Fired by the other player (in multiplayer).
    remote: bool,
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

/// One weapon firing, with the shooter's strength: what's sent to the
/// other player so they can replay it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Volley {
    pub weapon: Weapon,
    pub damage: f32,
    pub pellets: u32,
    pub blast_radius: f32,
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
pub struct Weapons {
    pub current: Weapon,
    /// The player's feet while a fight is on: the other player's shots
    /// can hurt them.
    pub victim: Option<Vec3>,
    cooldown: f32,
    shots: Vec<Shot>,
    blasts: Vec<Blast>,
    beams: Vec<Beam>,
    rng: u32,
    pub kills: u32,
    bolt_handles: Vec<usize>,
    rocket_handles: Vec<usize>,
    blast_handles: Vec<usize>,
    beam_handles: Vec<usize>,
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
            current: Weapon::Blaster,
            victim: None,
            cooldown: 0.0,
            shots: Vec::new(),
            blasts: Vec::new(),
            beams: Vec::new(),
            rng: 0x9e37_79b9,
            kills: 0,
            bolt_handles: pool(MAX_SHOTS, &bolt, &bolt_material),
            rocket_handles: pool(MAX_ROCKETS, &rocket, &rocket_material),
            blast_handles: pool(MAX_BLASTS, &blast, &blast_material),
            beam_handles: pool(MAX_BEAMS, &beam, &beam_material),
        }
    }

    #[cfg(test)]
    fn bare() -> Self {
        Self {
            current: Weapon::Blaster,
            victim: None,
            cooldown: 0.0,
            shots: Vec::new(),
            blasts: Vec::new(),
            beams: Vec::new(),
            rng: 1,
            kills: 0,
            bolt_handles: Vec::new(),
            rocket_handles: Vec::new(),
            blast_handles: Vec::new(),
            beam_handles: Vec::new(),
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
        if moves.weapons >= weapon.unlocked_at() && self.current != weapon {
            self.current = weapon;
            self.cooldown = self.cooldown.min(0.2);
            return true;
        }
        false
    }

    /// Fraction of the reload done (1 = ready).
    pub fn ready_fraction(&self, moves: &Moveset) -> f32 {
        1.0 - self.cooldown / (self.current.delay() / moves.fire_rate)
    }

    /// Ticks the reload; uses real time so slowing the world doesn't jam
    /// your gun.
    pub fn reload(&mut self, dt: f32) {
        self.cooldown = (self.cooldown - dt).max(0.0);
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
        if self.cooldown > 0.0 || moves.weapons < self.current.unlocked_at() {
            return (Events::default(), None);
        }
        self.cooldown = self.current.delay() / moves.fire_rate;
        let volley = Volley {
            weapon: self.current,
            damage: moves.weapon_damage,
            pellets: moves.pellets,
            blast_radius: moves.blast_radius,
        };
        let events = self.volley(&volley, shooters, false, authority, drones, colliders, terrain);
        (events, Some(volley))
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
                Weapon::Blaster => self.spawn(origin, dir * 90.0, Kind::Bolt, v.damage, 0.0, remote),
                Weapon::Scatter => {
                    let (a_axis, b_axis) = dir.any_orthonormal_pair();
                    for _ in 0..v.pellets.min(32) {
                        let a = self.random() * std::f32::consts::TAU;
                        let r = self.random().sqrt() * 0.08;
                        let d = (dir + (a_axis * a.cos() + b_axis * a.sin()) * r).normalize();
                        self.spawn(origin, d * 75.0, Kind::Bolt, v.damage, 0.0, remote);
                    }
                }
                Weapon::Rockets => self.spawn(origin, dir * 32.0, Kind::Rocket, 6.0 * v.damage, v.blast_radius, remote),
                Weapon::Railgun => {
                    let end = raycast(origin, dir, RAIL_RANGE, colliders, terrain)
                        .map_or(RAIL_RANGE, |p| p.distance(origin));
                    // The other player's beam through you.
                    if remote
                        && let Some(feet) = self.victim
                        && segment_hits_body(origin, dir, end, feet).is_some()
                    {
                        events.player_damage += 8.0 * v.damage * PVP_SCALE;
                    }
                    // Pierces every drone along the line.
                    if authority {
                        for (i, _) in drones.ray_hits(origin, dir, end) {
                            if let Some(at) = drones.damage(i, 8.0 * v.damage) {
                                events.credit(at, remote);
                            }
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

    /// Bookkeeping after hits: count your kills and blow up the drones.
    fn finish(&mut self, events: Events) -> Events {
        for &at in &events.destroyed {
            self.add_blast(at, 1.6);
        }
        self.kills += events.kills;
        events
    }

    /// Shows an explosion (also used for drones destroyed on the host).
    pub fn add_blast(&mut self, pos: Vec3, radius: f32) {
        if self.blasts.len() >= MAX_BLASTS {
            self.blasts.remove(0);
        }
        self.blasts.push(Blast { pos, radius, age: 0.0 });
    }

    fn spawn(&mut self, pos: Vec3, vel: Vec3, kind: Kind, damage: f32, blast: f32, remote: bool) {
        let (limit, count) = match kind {
            Kind::Bolt => (MAX_SHOTS, self.shots.iter().filter(|s| s.kind == Kind::Bolt).count()),
            Kind::Rocket => (MAX_ROCKETS, self.shots.iter().filter(|s| s.kind == Kind::Rocket).count()),
        };
        if count >= limit
            && let Some(oldest) = self.shots.iter().position(|s| s.kind == kind)
        {
            self.shots.remove(oldest);
        }
        self.shots.push(Shot { pos, vel, kind, damage, blast, life: SHOT_LIFE, remote });
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
        events: &mut Events,
    ) {
        if authority {
            for (i, dist) in drones.within(pos, radius) {
                let falloff = 1.0 - 0.6 * (dist / radius).min(1.0);
                if let Some(at) = drones.damage(i, damage * falloff) {
                    events.credit(at, remote);
                }
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

    /// A ground slam's shockwave: drones only, no shove.
    pub fn shockwave(&mut self, pos: Vec3, damage: f32, remote: bool, authority: bool, drones: &mut Drones) -> Events {
        let mut events = Events::default();
        self.explode(pos, 6.0, 4.0 * damage, remote, authority, drones, pos + Vec3::Y * 1000.0, &mut events);
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
        let mut i = 0;
        while i < self.shots.len() {
            let shot = &mut self.shots[i];
            shot.life -= dt;
            let travel = shot.vel * dt;
            let len = travel.length();
            let dir = travel.normalize_or(Vec3::X);
            let (pos, kind, damage, blast, remote, expired) =
                (shot.pos, shot.kind, shot.damage, shot.blast, shot.remote, shot.life <= 0.0);
            if len == 0.0 {
                // Frozen by Time Stop.
                i += 1;
                continue;
            }
            // The other player's shots can hit you (during a fight).
            let body = if remote { self.victim.and_then(|feet| segment_hits_body(pos, dir, len, feet)) } else { None };
            let drone = drones.ray_hit(pos, dir, len);
            let nearest_other = drone.map(|d| d.1).into_iter().chain(raycast(pos, dir, len, colliders, terrain).map(|p| p.distance(pos))).fold(f32::MAX, f32::min);
            if let Some(t) = body.filter(|t| *t <= nearest_other) {
                self.shots.remove(i);
                match kind {
                    Kind::Bolt => events.player_damage += damage * PVP_SCALE,
                    Kind::Rocket => self.explode(pos + dir * t, blast, damage, remote, authority, drones, player, &mut events),
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
                            if authority
                                && let Some(d) = target
                                && let Some(at) = drones.damage(d, damage)
                            {
                                events.credit(at, remote);
                            }
                        }
                        Kind::Rocket => self.explode(at, blast, damage, remote, authority, drones, player, &mut events),
                    }
                }
                None if expired => {
                    let shot = self.shots.remove(i);
                    if kind == Kind::Rocket {
                        self.explode(shot.pos, blast, damage, remote, authority, drones, player, &mut events);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abilities::{TREES, Tree};

    fn arsenal(level: u32) -> Moveset {
        let mut levels = [0; TREES];
        levels[Tree::Arsenal.index()] = level;
        Moveset::from_levels(levels)
    }

    fn drones_at(points: &[Vec3]) -> Drones {
        Drones::test_at(points)
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
        let volley = Volley { weapon: Weapon::Railgun, damage: 1.0, pellets: 8, blast_radius: 4.5 };

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
        let bolt = Volley { weapon: Weapon::Blaster, damage: 1.0, pellets: 8, blast_radius: 4.5 };

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