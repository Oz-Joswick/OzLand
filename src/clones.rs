//! Clones of the player (the Clone ability tree).
//!
//! Each clone is a full copy: the same animated character, its own physics,
//! the same abilities. A **stationary** clone stands where you left it. A
//! **mirror** clone is your reflection in an imaginary mirror placed in
//! front of you when it was made: step towards the mirror and it steps
//! towards you, strafe left and it strafes right, jump and it jumps.

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
/// How far in front of you the imaginary mirror stands.
const MIRROR_DISTANCE: f32 = 1.5;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Stationary,
    Mirror,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Mode::Stationary => "stationary",
            Mode::Mirror => "mirror",
        }
    }
}

struct Clone {
    player: Player,
    yaw: f32,
    mode: Mode,
    /// The mirror's normal (your facing when the clone was made).
    mirror_normal: Vec3,
    body: usize,
}

pub struct Clones {
    bodies: Vec<Body>,
    active: Vec<Clone>,
    /// The kind of clone the next one will be.
    pub mode: Mode,
}

/// Reflects a heading (yaw) in a vertical mirror with the given normal.
fn mirror_yaw(yaw: f32, normal: Vec3) -> f32 {
    let f = Vec3::new(yaw.cos(), 0.0, yaw.sin());
    let r = f - 2.0 * f.dot(normal) * normal;
    r.z.atan2(r.x)
}

impl Clones {
    pub fn new(renderer: &mut Renderer, model: Option<Rc<AnimatedModel>>) -> Self {
        Self {
            bodies: (0..MAX_CLONES).map(|_| Body::new(renderer, model.clone())).collect(),
            active: Vec::new(),
            mode: Mode::Stationary,
        }
    }

    /// Paints every clone in the player's colour.
    pub fn set_color(&mut self, color: Vec3) {
        for b in &mut self.bodies {
            b.set_color(color);
        }
    }

    pub fn count(&self) -> usize {
        self.active.len()
    }

    /// Makes a clone of `player`; past `max`, the oldest clone disappears.
    pub fn spawn(&mut self, player: &Player, yaw: f32, max: usize, renderer: &mut Renderer) {
        if max == 0 {
            return;
        }
        while self.active.len() >= max {
            let old = self.active.remove(0);
            self.bodies[old.body].hide(renderer);
        }
        let free = (0..MAX_CLONES)
            .find(|b| !self.active.iter().any(|c| c.body == *b))
            .expect("a free body");
        let facing = Vec3::new(yaw.cos(), 0.0, yaw.sin());
        let (pos, clone_yaw) = match self.mode {
            Mode::Stationary => (player.pos, yaw),
            // Your reflection stands as far behind the mirror as you are in front.
            Mode::Mirror => (player.pos + facing * (2.0 * MIRROR_DISTANCE), mirror_yaw(yaw, facing)),
        };
        self.bodies[free].reset_motion();
        self.active.push(Clone {
            player: Player::new(pos),
            yaw: clone_yaw,
            mode: self.mode,
            mirror_normal: facing,
            body: free,
        });
    }

    /// The clones' bodies, e.g. for bounce pads.
    pub fn players_mut(&mut self) -> impl Iterator<Item = &mut Player> {
        self.active.iter_mut().map(|c| &mut c.player)
    }

    /// Each clone's position and heading (for firing with you).
    pub fn shooters(&self) -> Vec<(Vec3, f32)> {
        self.active.iter().map(|c| (c.player.pos, c.yaw)).collect()
    }

    pub fn dismiss_all(&mut self, renderer: &mut Renderer) {
        for c in self.active.drain(..) {
            self.bodies[c.body].hide(renderer);
        }
    }

    /// Swaps places between `player` and the newest clone.
    pub fn switch_with_newest(&mut self, player: &mut Player) -> bool {
        let Some(clone) = self.active.last_mut() else {
            return false;
        };
        let (a, b) = (player.pos, clone.player.pos);
        player.teleport(b);
        clone.player.teleport(a);
        self.bodies[clone.body].reset_motion();
        true
    }

    /// Moves every clone for this frame. `controls` and `yaw` are the
    /// player's, which mirror clones copy (reflected).
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
    ) {
        for clone in &mut self.active {
            let input = match clone.mode {
                Mode::Stationary => Controls::default(),
                Mode::Mirror => {
                    // A reflection keeps forward/back (relative to its own
                    // facing) but swaps left and right.
                    clone.yaw = mirror_yaw(yaw, clone.mirror_normal);
                    Controls {
                        right: -controls.right,
                        ..*controls
                    }
                }
            };
            clone.player.update(dt, &input, clone.yaw, colliders, terrain, moves);
            let p = &clone.player;
            self.bodies[clone.body].update(renderer, dt, p.pos, clone.yaw, p.grounded(), p.flight_direction(), true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirror_reflects_headings() {
        // Mirror facing +X: looking at it, you see yourself looking back.
        let n = Vec3::X;
        assert!((mirror_yaw(0.0, n).abs() - std::f32::consts::PI).abs() < 1e-4);
        // Looking sideways along the mirror stays the same.
        let side = std::f32::consts::FRAC_PI_2;
        assert!((mirror_yaw(side, n) - side).abs() < 1e-4);
    }
}
