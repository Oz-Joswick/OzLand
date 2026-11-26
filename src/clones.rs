//! Clones of the player (the Clone ability tree).
//!
//! Two kinds:
//!
//! - **Clones** (the Clone skill) look just like you and mirror your
//!   movement: the same animated character with its own physics. They
//!   appear with you in a circle, everyone facing its middle (see
//!   [`formation`]), and turn as you turn. They last until destroyed or
//!   dismissed.
//! - **Arcane clones** (the Arcane Clone spell) glow violet and come in
//!   pairs that stand either side of you, facing where you face, mirroring
//!   every attack and spell you make from where they stand. Each fades
//!   after five of your actions.
//!
//! Drones can destroy either kind.

use std::rc::Rc;

use glam::Vec3;

use crate::abilities::Moveset;
use crate::animation::AnimatedModel;
use crate::body::Body;
use crate::collision::Aabb;
use crate::player::{Controls, Player};
use crate::renderer::Renderer;
use crate::terrain::Terrain;

/// Most clones that can exist at once (Clone Squad plus Ascension).
pub const MAX_CLONES: usize = 8;
/// A clone's health, and how many of your actions an arcane clone copies
/// before fading.
pub const CLONE_HEALTH: f32 = 40.0;
pub const CLONE_ACTIONS: u32 = 5;
/// Arcane clones: how far to your side the first pair stands, how much
/// further out each next pair, and how far back (just behind your shoulder).
const BESIDE: f32 = 2.0;
const SPACING: f32 = 1.6;
const BACK: f32 = 0.4;
const ARCANE_COLOR: Vec3 = Vec3::new(0.62, 0.3, 1.0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Looks like you and mirrors your movement.
    Mirror,
    /// Stands beside you and mirrors your attacks and spells.
    Arcane,
}

struct Clone {
    kind: Kind,
    player: Player,
    yaw: f32,
    /// Facing relative to yours (towards the circle's middle when it spawned).
    heading_offset: f32,
    body: usize,
    health: f32,
    /// Your actions it has copied so far.
    actions: u32,
}

pub struct Clones {
    bodies: Vec<Body>,
    active: Vec<Clone>,
    /// Your colour (mirror clones wear it).
    color: Vec3,
}

/// Where you and `clones` clones stand in a circle in front of you, and
/// which way each faces (its middle). You are on the circle already; the
/// clones take the other places, spaced evenly. `ground` gives the
/// ground's height for a spot; on a roof or in the air they appear at
/// your height instead.
pub fn formation(you: Vec3, yaw: f32, clones: usize, ground: impl Fn(f32, f32) -> f32) -> Vec<(Vec3, f32)> {
    let total = clones + 1;
    // Big enough for everyone to have about 2 m of room.
    let radius = (2.0 * total as f32 / std::f32::consts::TAU).max(2.5);
    let forward = Vec3::new(yaw.cos(), 0.0, yaw.sin());
    let middle = you + forward * radius;
    let on_ground = you.y - ground(you.x, you.z) < 0.5;
    let start = (-forward.z).atan2(-forward.x);
    (1..total)
        .map(|k| {
            let a = start + k as f32 * std::f32::consts::TAU / total as f32;
            let mut pos = middle + Vec3::new(a.cos(), 0.0, a.sin()) * radius;
            pos.y = if on_ground { ground(pos.x, pos.z) } else { you.y };
            let to_middle = middle - pos;
            (pos, to_middle.z.atan2(to_middle.x))
        })
        .collect()
}

impl Clones {
    pub fn new(renderer: &mut Renderer, model: Option<Rc<AnimatedModel>>) -> Self {
        Self {
            bodies: (0..MAX_CLONES).map(|_| Body::new(renderer, model.clone())).collect(),
            active: Vec::new(),
            color: Vec3::ONE,
        }
    }

    /// Paints the mirror clones in the player's colour (arcane ones stay violet).
    pub fn set_color(&mut self, color: Vec3) {
        self.color = color;
        for (i, b) in self.bodies.iter_mut().enumerate() {
            let arcane = self.active.iter().any(|c| c.body == i && c.kind == Kind::Arcane);
            b.set_color(if arcane { ARCANE_COLOR } else { color });
        }
    }

    /// How many of all kinds.
    pub fn count(&self) -> usize {
        self.active.len()
    }

    /// How many of one kind.
    pub fn count_of(&self, kind: Kind) -> usize {
        self.active.iter().filter(|c| c.kind == kind).count()
    }

    /// Makes a clone of `kind` at `pos` facing `yaw` (you face `your_yaw`);
    /// past `max` of that kind, the oldest of them disappears.
    pub fn spawn(&mut self, kind: Kind, pos: Vec3, yaw: f32, your_yaw: f32, max: usize, renderer: &mut Renderer) {
        if max == 0 {
            return;
        }
        while self.count_of(kind) >= max {
            let Some(i) = self.active.iter().position(|c| c.kind == kind) else { break };
            let old = self.active.remove(i);
            self.bodies[old.body].hide(renderer);
        }
        // Every body in use: the oldest clone of any kind makes way.
        if self.active.len() >= MAX_CLONES {
            let old = self.active.remove(0);
            self.bodies[old.body].hide(renderer);
        }
        let free = (0..MAX_CLONES)
            .find(|b| !self.active.iter().any(|c| c.body == *b))
            .expect("a free body");
        self.bodies[free].reset_motion();
        self.bodies[free].set_color(if kind == Kind::Arcane { ARCANE_COLOR } else { self.color });
        self.active.push(Clone {
            kind,
            player: Player::new(pos),
            yaw,
            heading_offset: yaw - your_yaw,
            body: free,
            health: CLONE_HEALTH,
            actions: 0,
        });
    }

    /// Where the `k`th arcane clone stands: the first two on your left and
    /// right, the next two a step further out, and so on.
    pub fn beside(you: Vec3, yaw: f32, k: usize) -> Vec3 {
        let forward = Vec3::new(yaw.cos(), 0.0, yaw.sin());
        let side = Vec3::new(-forward.z, 0.0, forward.x);
        let sign = if k % 2 == 0 { -1.0 } else { 1.0 };
        let out = BESIDE + (k / 2) as f32 * SPACING;
        you - forward * BACK + side * (sign * out)
    }

    /// Moves the mirror clones (oldest first) to `spots` from
    /// [`formation`], each facing as given.
    pub fn arrange(&mut self, spots: &[(Vec3, f32)], your_yaw: f32) {
        for (clone, &(pos, yaw)) in self.active.iter_mut().filter(|c| c.kind == Kind::Mirror).zip(spots) {
            clone.player.teleport(pos);
            clone.yaw = yaw;
            clone.heading_offset = yaw - your_yaw;
            self.bodies[clone.body].reset_motion();
        }
    }

    /// Every arcane clone copies one of your actions; those that have done
    /// their five fade. Returns how many faded.
    pub fn spend_action(&mut self, renderer: &mut Renderer) -> usize {
        for c in self.active.iter_mut().filter(|c| c.kind == Kind::Arcane) {
            c.actions += 1;
        }
        self.remove_where(renderer, |c| c.kind == Kind::Arcane && c.actions >= CLONE_ACTIONS)
    }

    /// Damage to clones (by index, as in [`Clones::middles`]). Returns how
    /// many were destroyed.
    pub fn damage(&mut self, hits: &[(usize, f32)], renderer: &mut Renderer) -> usize {
        for &(i, amount) in hits {
            if let Some(c) = self.active.get_mut(i) {
                c.health -= amount;
            }
        }
        self.remove_where(renderer, |c| c.health <= 0.0)
    }

    fn remove_where(&mut self, renderer: &mut Renderer, gone: impl Fn(&Clone) -> bool) -> usize {
        let before = self.active.len();
        let bodies = &mut self.bodies;
        self.active.retain(|c| {
            let keep = !gone(c);
            if !keep {
                bodies[c.body].hide(renderer);
            }
            keep
        });
        before - self.active.len()
    }

    /// Each clone's middle (what drones aim at).
    pub fn middles(&self) -> Vec<Vec3> {
        self.active.iter().map(|c| c.player.pos + Vec3::Y * 0.9).collect()
    }

    /// Health bars for hurt clones: (above its head, health left 0..1).
    pub fn bars(&self) -> Vec<(Vec3, f32)> {
        self.active
            .iter()
            .filter(|c| c.health < CLONE_HEALTH)
            .map(|c| (c.player.pos + Vec3::Y * 1.0, c.health / CLONE_HEALTH))
            .collect()
    }

    /// The clones' bodies, e.g. for bounce pads.
    pub fn players_mut(&mut self) -> impl Iterator<Item = &mut Player> {
        self.active.iter_mut().map(|c| &mut c.player)
    }

    /// Each mirror clone's position and heading (for firing with you, with
    /// Legion).
    pub fn shooters(&self) -> Vec<(Vec3, f32)> {
        self.active.iter().filter(|c| c.kind == Kind::Mirror).map(|c| (c.player.pos, c.yaw)).collect()
    }

    /// Where each arcane clone stands (they copy your attacks and spells).
    pub fn arcane(&self) -> Vec<Vec3> {
        self.active.iter().filter(|c| c.kind == Kind::Arcane).map(|c| c.player.pos).collect()
    }

    pub fn dismiss_all(&mut self, renderer: &mut Renderer) {
        for c in self.active.drain(..) {
            self.bodies[c.body].hide(renderer);
        }
    }

    /// Swaps places between `player` and the newest mirror clone (arcane
    /// clones glide back beside you, so swapping with one does nothing).
    pub fn switch_with_newest(&mut self, player: &mut Player) -> bool {
        let Some(clone) = self.active.iter_mut().rev().find(|c| c.kind == Kind::Mirror) else {
            return false;
        };
        let (a, b) = (player.pos, clone.player.pos);
        player.teleport(b);
        clone.player.teleport(a);
        self.bodies[clone.body].reset_motion();
        true
    }

    /// Moves every clone for this frame: mirror clones reflect your strafe
    /// input; arcane ones follow behind `you` (where you are, and whether
    /// you're on the ground).
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        renderer: &mut Renderer,
        dt: f32,
        controls: &Controls,
        yaw: f32,
        colliders: &[Aabb],
        terrain: &Terrain,
        moves: &Moveset,
        you: Option<(Vec3, bool)>,
    ) {
        let mut k = 0;
        for clone in &mut self.active {
            match clone.kind {
                Kind::Mirror => {
                    clone.yaw = yaw + clone.heading_offset;
                    let input = Controls {
                        right: -controls.right,
                        ..*controls
                    };
                    clone.player.update(dt, &input, clone.yaw, colliders, terrain, moves);
                    let p = &clone.player;
                    self.bodies[clone.body].update(renderer, dt, p.pos, clone.yaw, p.grounded(), p.flight_direction(), true);
                }
                Kind::Arcane => {
                    // Glides to its place beside you, facing where you face.
                    clone.yaw = yaw;
                    if let Some((pos, grounded)) = you {
                        let mut spot = Self::beside(pos, yaw, k);
                        // On the ground: the terrain beside you, unless you
                        // stand on a roof or ledge well above it (then at
                        // your height, not inside the building below).
                        let ground = terrain.height_at(spot.x, spot.z);
                        spot.y = if grounded && pos.y - ground < 1.5 { ground } else { pos.y.max(ground) };
                        let p = clone.player.pos + (spot - clone.player.pos) * (dt * 8.0).min(1.0);
                        clone.player.teleport(p);
                    }
                    k += 1;
                    let p = &clone.player;
                    self.bodies[clone.body].update(renderer, dt, p.pos, clone.yaw, true, None, true);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clones_stand_round_a_circle_facing_its_middle() {
        let you = Vec3::ZERO;
        let spots = formation(you, 0.0, 3, |_, _| 0.0); // facing +X
        assert_eq!(spots.len(), 3);
        let radius = 2.5;
        let middle = Vec3::new(radius, 0.0, 0.0);
        for &(pos, yaw) in &spots {
            assert!((pos.distance(middle) - radius).abs() < 1e-3, "on the circle: {pos:?}");
            let facing = Vec3::new(yaw.cos(), 0.0, yaw.sin());
            assert!(facing.dot((middle - pos).normalize()) > 0.999, "facing the middle");
        }
        // Evenly spaced with you: four places, a quarter turn apart.
        assert!((spots[1].0 - Vec3::new(2.0 * radius, 0.0, 0.0)).length() < 1e-3, "opposite you");
    }

    #[test]
    fn arcane_clones_stand_either_side_of_you() {
        // Facing +X: the first pair at z = -2 and +2, just behind your shoulder.
        let (left, right) = (Clones::beside(Vec3::ZERO, 0.0, 0), Clones::beside(Vec3::ZERO, 0.0, 1));
        assert!((left.z + BESIDE).abs() < 1e-4 && (right.z - BESIDE).abs() < 1e-4, "one each side");
        assert!((left.x + BACK).abs() < 1e-4 && (right.x + BACK).abs() < 1e-4, "level with you, a touch back");
        let (far_left, far_right) = (Clones::beside(Vec3::ZERO, 0.0, 2), Clones::beside(Vec3::ZERO, 0.0, 3));
        assert!(far_left.z < left.z && far_right.z > right.z, "the second pair further out");
    }
}
