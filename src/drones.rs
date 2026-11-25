//! Practice drones: floating targets for the weapons. They circle a home
//! spot, flinch when hit, drift towards you when you're near, and respawn a
//! while after being destroyed. Each one destroyed is worth experience.

use glam::{Mat4, Quat, Vec2, Vec3, Vec4};

use crate::mesh::MeshData;
use crate::model::MaterialData;
use crate::renderer::Renderer;
use crate::terrain::Terrain;

pub const DRONE_RADIUS: f32 = 0.7;
const MAX_HEALTH: f32 = 6.0;
const RESPAWN: f32 = 25.0;
const ORBIT: f32 = 5.0;

struct Drone {
    home: Vec3,
    pos: Vec3,
    health: f32,
    /// Seconds until it comes back (when destroyed).
    respawn: f32,
    phase: f32,
    /// Time since last hit (drives the flinch and the faster circling).
    since_hit: f32,
    /// Where the host says it is (multiplayer joiner).
    net: Vec3,
}

impl Drone {
    fn alive(&self) -> bool {
        self.health > 0.0
    }
}

pub struct Drones {
    drones: Vec<Drone>,
    /// Two meshes per drone: the dark shell and its glowing eye ring.
    handles: Vec<(usize, usize)>,
    time: f32,
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

/// Cheap deterministic hash to scatter the drones.
fn hash(i: u32, salt: u32) -> f32 {
    let mut x = i.wrapping_mul(747_796_405).wrapping_add(salt.wrapping_mul(2_891_336_453));
    x = ((x >> ((x >> 28) + 4)) ^ x).wrapping_mul(277_803_737);
    ((x >> 22) ^ x) as f32 / u32::MAX as f32
}

/// Home spots: a few near the start, the rest scattered over the valley.
fn homes(terrain: &Terrain, start: Vec3, count: u32) -> Vec<Vec3> {
    let lift = |x: f32, z: f32| {
        let y = terrain.height_at(x, z).max(terrain.water_level) + 6.5; // above the rooftops
        Vec3::new(x, y, z)
    };
    let mut out: Vec<Vec3> = [(14.0, -10.0), (-16.0, 6.0), (6.0, 18.0)]
        .iter()
        .map(|&(dx, dz)| lift(start.x + dx, start.z + dz))
        .collect();
    let reach = terrain.half_size - 30.0;
    for i in 0..count.saturating_sub(3) {
        let x = (hash(i, 1) * 2.0 - 1.0) * reach;
        let z = (hash(i, 2) * 2.0 - 1.0) * reach;
        out.push(lift(x, z));
    }
    out
}

impl Drones {
    pub fn new(renderer: &mut Renderer, terrain: &Terrain, start: Vec3) -> Self {
        let shell = sphere_mesh(DRONE_RADIUS, 8, 12);
        let shell_material = MaterialData {
            base_color: Vec4::new(0.16, 0.18, 0.22, 1.0),
            specular: 1.2,
            shininess: 60.0,
            ground_occlusion: 0.0,
            ..Default::default()
        };
        let mut ring = MeshData::default();
        ring.push_cylinder(Vec3::new(0.0, -0.06, 0.0), Vec3::Y, DRONE_RADIUS + 0.08, 0.12, 20, Vec4::ONE, 1.0);
        let ring_material = MaterialData {
            base_color: Vec4::new(2.8, 0.35, 0.25, 1.0),
            ground_occlusion: 0.0,
            ..Default::default()
        };
        let drones: Vec<Drone> = homes(terrain, start, 20)
            .into_iter()
            .enumerate()
            .map(|(i, home)| Drone {
                home,
                pos: home,
                health: MAX_HEALTH,
                respawn: 0.0,
                phase: i as f32 * 1.7,
                since_hit: 10.0,
                net: home,
            })
            .collect();
        let handles = drones
            .iter()
            .map(|_| {
                (
                    renderer.add_dynamic_mesh(&shell, &shell_material),
                    renderer.add_dynamic_mesh(&ring, &ring_material),
                )
            })
            .collect();
        Self { drones, handles, time: 0.0 }
    }

    #[cfg(test)]
    pub(crate) fn test_at(positions: &[Vec3]) -> Self {
        let drones = positions
            .iter()
            .map(|&p| Drone { home: p, pos: p, health: MAX_HEALTH, respawn: 0.0, phase: 0.0, since_hit: 10.0, net: p })
            .collect();
        Self { drones, handles: Vec::new(), time: 0.0 }
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
            } else if !d.alive() && alive {
                d.health = MAX_HEALTH;
                d.pos = pos;
            }
            d.net = pos;
        }
        downed
    }

    /// Moves the drones (circling home; drifting closer when the player is
    /// near) and draws them. `dt` is world time (slowed by Time powers).
    /// Without `simulate` (a multiplayer joiner) they glide to where the
    /// host says they are instead.
    pub fn update(&mut self, renderer: &mut Renderer, dt: f32, player: Vec3, simulate: bool) {
        self.time += dt;
        for (i, d) in self.drones.iter_mut().enumerate() {
            if !simulate {
                d.pos += (d.net - d.pos) * (dt * 10.0).min(1.0);
                d.since_hit += dt;
            }
            if simulate {
                d.since_hit += dt;
            }
            if simulate && !d.alive() {
                d.respawn -= dt;
                if d.respawn <= 0.0 {
                    d.health = MAX_HEALTH;
                    d.pos = d.home;
                }
            }
            if simulate && d.alive() {
                let spin = if d.since_hit < 2.0 { 1.6 } else { 0.45 };
                d.phase += dt * spin;
                let mut target = d.home
                    + Vec3::new(d.phase.cos() * ORBIT, (self.time * 1.3 + i as f32).sin() * 0.6, d.phase.sin() * ORBIT);
                // Curious: drift partway towards a nearby player.
                let to_player = player + Vec3::Y * 2.5 - target;
                if to_player.length() < 30.0 {
                    target += to_player * 0.35;
                }
                d.pos += (target - d.pos) * (dt * 2.0).min(1.0);
            }
            if let Some(&(shell, ring)) = self.handles.get(i) {
                let flinch = 1.0 + (0.25 - d.since_hit).max(0.0) * 1.2;
                let t = Mat4::from_scale_rotation_translation(
                    Vec3::splat(flinch),
                    Quat::from_rotation_y(self.time * 2.0 + i as f32),
                    d.pos,
                );
                renderer.set_transform(shell, t, d.alive());
                renderer.set_transform(ring, t, d.alive());
            }
        }
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
                let r2 = DRONE_RADIUS * DRONE_RADIUS;
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

    /// Drones within `radius` of `center`: (index, distance).
    pub fn within(&self, center: Vec3, radius: f32) -> Vec<(usize, f32)> {
        self.drones
            .iter()
            .enumerate()
            .filter(|(_, d)| d.alive())
            .map(|(i, d)| (i, d.pos.distance(center)))
            .filter(|&(_, dist)| dist < radius + DRONE_RADIUS)
            .collect()
    }

    /// Damages a drone; returns where it was if this destroyed it.
    pub fn damage(&mut self, index: usize, amount: f32) -> Option<Vec3> {
        let d = self.drones.get_mut(index)?;
        if !d.alive() {
            return None;
        }
        d.health -= amount;
        d.since_hit = 0.0;
        if d.alive() {
            return None;
        }
        d.respawn = RESPAWN;
        Some(d.pos)
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
        assert!((hits[0].1 - (5.0 - (DRONE_RADIUS * DRONE_RADIUS - 0.04).sqrt())).abs() < 1e-4);
        assert!(d.ray_hit(Vec3::ZERO, Vec3::X, 3.0).is_none(), "out of range");
        assert!(d.damage(1, 4.0).is_none());
        assert!(d.damage(1, 4.0).is_some(), "destroyed");
        assert_eq!(d.alive_count(), 2);
        assert!(d.ray_hits(Vec3::ZERO, Vec3::X, 50.0).iter().all(|h| h.0 != 1), "dead drones aren't hit");
        assert_eq!(d.within(Vec3::new(5.0, 4.0, 0.0), 1.0).len(), 1);
    }
}
