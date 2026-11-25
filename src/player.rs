//! Player movement and collision against static boxes and the terrain.
//!
//! The base model: axis-by-axis collision resolution (so you slide along
//! walls), automatic step-up onto low ledges, gravity and jumping. On top of
//! that sit the abilities from `abilities.rs`: extra jumps, elytra-style
//! gliding, wall running and jumping, mantling, sliding and bunny hopping,
//! hovering, levitation and free flight, and being pulled by a grapple.

use glam::Vec3;

use crate::abilities::Moveset;
use crate::collision::Aabb;
use crate::terrain::Terrain;

/// How long one rocket boost pushes, in seconds.
const ROCKET_TIME: f32 = 1.1;
/// Gliding is simulated in Minecraft's units (blocks per tick, 20 ticks a
/// second; a block is a meter), so its elytra rules carry over unchanged.
const TICKS: f32 = 20.0;
/// Longest move per collision check when moving fast.
const MOVE_STEP: f32 = 0.25;
/// Downward speed of a ground slam.
const SLAM_SPEED: f32 = 30.0;
/// Gravity multiplier with Low Gravity on.
const LOW_GRAVITY: f32 = 0.4;
/// Wall slide fall speed, wall climb speed and the push of a wall jump.
const WALL_SLIDE_SPEED: f32 = 3.0;
const WALL_CLIMB_SPEED: f32 = 5.5;
const WALL_JUMP_PUSH: f32 = 7.0;
/// Levitation rise speed and free flight speed.
const LEVITATE_SPEED: f32 = 6.0;
const FLY_SPEED: f32 = 14.0;
/// A jump this soon after landing counts as a bunny hop.
const BUNNY_HOP_WINDOW: f32 = 0.2;
/// The grapple's top speed, and how close counts as arrived.
const HOOK_MAX_SPEED: f32 = 45.0;
const HOOK_ARRIVED: f32 = 1.3;

/// Steepest terrain you can walk up (rise over run; 1.2 ≈ 50°).
pub const MAX_SLOPE: f32 = 1.2;
/// Deepest water you can wade into.
pub const MAX_WADE_DEPTH: f32 = 0.6;
/// How far from the map edge you're stopped.
pub const EDGE_MARGIN: f32 = 6.0;
/// How far down the player sticks to the ground when walking downhill.
const STICK_DOWN: f32 = 0.6;

pub const MOVE_SPEED: f32 = 5.0;
pub const SPRINT_MULTIPLIER: f32 = 1.8;
pub const GRAVITY: f32 = 22.0;
pub const JUMP_SPEED: f32 = 8.0;

pub const PLAYER_RADIUS: f32 = 0.3;
pub const PLAYER_HEIGHT: f32 = 1.8;
pub const EYE_HEIGHT: f32 = 1.6;
pub const STEP_HEIGHT: f32 = 0.55;

/// What the player wants to do this frame, decoupled from the key bindings.
#[derive(Clone, Copy, Default)]
pub struct Controls {
    /// -1..1, positive = forward.
    pub forward: f32,
    /// -1..1, positive = strafe right.
    pub right: f32,
    pub sprint: bool,
    /// Jump pressed this frame (in mid-air: wall jump, extra jump, or
    /// spread/fold the wings).
    pub jump: bool,
    /// Where the camera looks up/down, in radians (positive = up). Steers
    /// the glide and free flight.
    pub pitch: f32,
    /// Ctrl pressed this frame: slam in the air, slide on the ground.
    pub slam: bool,
    /// Rocket boost pressed this frame (while gliding).
    pub boost: bool,
    /// Held: hover, levitate or fly (Gravity tree).
    pub levitate: bool,
    /// Toggle low gravity this frame.
    pub gravity_toggle: bool,
}

pub struct Player {
    /// Position of the feet.
    pub pos: Vec3,
    vel_y: f32,
    grounded: bool,
    air_jumps_used: u32,
    boosts_used: u32,
    gliding: bool,
    slamming: bool,
    /// Velocity while gliding (m/s).
    flight: Vec3,
    /// Time left on the current rocket boost.
    rocket: f32,
    /// Horizontal velocity (carried through jumps and into a glide).
    ground_velocity: Vec3,
    low_gravity: bool,
    /// Seconds of hover/levitation left.
    hover_left: f32,
    wall_run_left: f32,
    wall_running: bool,
    /// Speed multiplier built by sliding, bunny hops and run-ups (>= 1).
    momentum: f32,
    slide_left: f32,
    slide_dir: Vec3,
    /// Time standing on the ground (0 while airborne).
    since_landed: f32,
    /// Grapple point being pulled towards.
    hook: Option<Vec3>,
    flying: bool,
    /// Hovering or levitating this frame.
    hovering: bool,
}

impl Player {
    pub fn new(pos: Vec3) -> Self {
        Self {
            pos,
            vel_y: 0.0,
            grounded: true,
            air_jumps_used: 0,
            boosts_used: 0,
            gliding: false,
            slamming: false,
            flight: Vec3::ZERO,
            rocket: 0.0,
            ground_velocity: Vec3::ZERO,
            low_gravity: false,
            hover_left: 0.0,
            wall_run_left: 0.0,
            wall_running: false,
            momentum: 1.0,
            slide_left: 0.0,
            slide_dir: Vec3::ZERO,
            since_landed: 0.0,
            hook: None,
            flying: false,
            hovering: false,
        }
    }

    /// Standing on something (not jumping or falling).
    pub fn grounded(&self) -> bool {
        self.grounded
    }

    /// Direction of travel while gliding or flying (the body leans into it).
    pub fn flight_direction(&self) -> Option<Vec3> {
        if self.gliding {
            Some(self.flight.normalize_or(Vec3::NEG_Y))
        } else if self.flying && self.ground_velocity.length() > 1.0 {
            Some((self.ground_velocity + Vec3::Y * self.vel_y).normalize())
        } else {
            None
        }
    }

    /// Current velocity, m/s.
    pub fn velocity(&self) -> Vec3 {
        if self.gliding {
            self.flight
        } else {
            self.ground_velocity + Vec3::Y * self.vel_y
        }
    }

    /// Glide boosts used since last touching the ground.
    pub fn boosts_used(&self) -> u32 {
        self.boosts_used
    }

    pub fn low_gravity(&self) -> bool {
        self.low_gravity
    }

    pub fn hover_left(&self) -> f32 {
        self.hover_left
    }

    pub fn momentum(&self) -> f32 {
        self.momentum
    }

    pub fn gliding(&self) -> bool {
        self.gliding
    }

    pub fn hovering(&self) -> bool {
        self.hovering
    }

    pub fn wall_running(&self) -> bool {
        self.wall_running
    }

    pub fn hooked(&self) -> bool {
        self.hook.is_some()
    }

    /// Moves the player instantly (switching places, recall, rewind).
    pub fn teleport(&mut self, pos: Vec3) {
        self.pos = pos;
        self.vel_y = 0.0;
        self.flight = Vec3::ZERO;
        self.ground_velocity = Vec3::ZERO;
        self.gliding = false;
        self.slamming = false;
        self.hook = None;
    }

    /// Thrown upwards (a bounce pad).
    pub fn launch(&mut self, up: f32) {
        self.vel_y = up;
        self.grounded = false;
        self.gliding = false;
        self.slamming = false;
    }

    /// Shoved by a blast (rocket jumps).
    pub fn push(&mut self, impulse: Vec3) {
        if impulse == Vec3::ZERO {
            return;
        }
        if self.gliding {
            self.flight += impulse;
            return;
        }
        self.ground_velocity += Vec3::new(impulse.x, 0.0, impulse.z);
        self.vel_y = self.vel_y.max(0.0) + impulse.y;
        if impulse.y > 0.0 {
            self.grounded = false;
            self.slamming = false;
        }
    }

    /// Air jumps, boosts and hover energy back to full.
    pub fn refill(&mut self, moves: &Moveset) {
        self.air_jumps_used = 0;
        self.boosts_used = 0;
        self.hover_left = moves.hover_time;
        self.wall_run_left = moves.wall_run;
    }

    /// Starts being pulled towards a grapple point.
    pub fn set_hook(&mut self, point: Vec3) {
        self.hook = Some(point);
        self.gliding = false;
        self.slamming = false;
        self.slide_left = 0.0;
    }

    /// Lets go of the grapple. With `fling` (Slingshot) you're thrown on
    /// faster and your air abilities refill.
    pub fn release_hook(&mut self, fling: bool, moves: &Moveset) {
        if self.hook.take().is_none() {
            return;
        }
        if fling {
            self.ground_velocity *= moves.fling;
            self.vel_y = self.vel_y.max(0.0) * moves.fling + 5.0;
            self.refill(moves);
        }
    }

    /// Phase Shift: blinks along `direction` in 3D (up into the sky, down
    /// a cliff), stopping short of anything solid and never into the
    /// ground. Returns false if there was no room to move.
    pub fn blink_free(&mut self, direction: Vec3, distance: f32, colliders: &[Aabb], terrain: &Terrain) -> bool {
        let dir = direction.normalize_or_zero();
        if dir == Vec3::ZERO {
            return false;
        }
        let limit = terrain.half_size - EDGE_MARGIN;
        let mut best = self.pos;
        let mut travelled = 0.0;
        while travelled < distance {
            travelled = (travelled + 0.25).min(distance);
            let mut candidate = self.pos + dir * travelled;
            if candidate.x.abs() > limit || candidate.z.abs() > limit {
                break;
            }
            candidate.y = candidate.y.max(terrain.height_at(candidate.x, candidate.z));
            if colliders.iter().any(|c| c.intersects(&Self::bounds_at(candidate))) {
                break;
            }
            best = candidate;
        }
        let moved = best != self.pos;
        self.pos = best;
        self.vel_y = 0.0; // hang for a moment at the end
        self.grounded = self.pos.y <= terrain.height_at(self.pos.x, self.pos.z) + 1e-3;
        moved
    }

    /// Teleport dash: blinks up to distance along direction (flattened),
    /// stopping short of anything solid or a cliff. Returns false if there
    /// was no room to move.
    pub fn blink(&mut self, direction: Vec3, distance: f32, colliders: &[Aabb], terrain: &Terrain) -> bool {
        let dir = Vec3::new(direction.x, 0.0, direction.z).normalize_or_zero();
        if dir == Vec3::ZERO {
            return false;
        }
        let limit = terrain.half_size - EDGE_MARGIN;
        let mut best = self.pos;
        let mut travelled = 0.0;
        while travelled < distance {
            travelled = (travelled + 0.25).min(distance);
            let mut candidate = self.pos + dir * travelled;
            let ground = terrain.height_at(candidate.x, candidate.z);
            if ground - self.pos.y > 2.0 || candidate.x.abs() > limit || candidate.z.abs() > limit {
                break; // cliff or map edge
            }
            candidate.y = candidate.y.max(ground);
            if colliders.iter().any(|c| c.intersects(&Self::bounds_at(candidate))) {
                break;
            }
            best = candidate;
        }
        let moved = best != self.pos;
        self.pos = best;
        self.vel_y = self.vel_y.max(0.0);
        moved
    }

    pub fn eye(&self) -> Vec3 {
        self.pos + Vec3::new(0.0, EYE_HEIGHT, 0.0)
    }

    fn bounds_at(pos: Vec3) -> Aabb {
        Aabb {
            min: Vec3::new(pos.x - PLAYER_RADIUS, pos.y, pos.z - PLAYER_RADIUS),
            max: Vec3::new(pos.x + PLAYER_RADIUS, pos.y + PLAYER_HEIGHT, pos.z + PLAYER_RADIUS),
        }
    }

    /// The wall right beside you, if any, as the direction pointing away
    /// from it. Only things taller than a step count.
    fn touching_wall(&self, colliders: &[Aabb]) -> Option<Vec3> {
        for dir in [Vec3::X, Vec3::NEG_X, Vec3::Z, Vec3::NEG_Z] {
            let mut probe = Self::bounds_at(self.pos + dir * 0.1);
            probe.min.y += 0.3; // not the floor
            if colliders
                .iter()
                .any(|c| c.intersects(&probe) && c.max.y > self.pos.y + 0.6)
            {
                return Some(-dir);
            }
        }
        None
    }

    /// True if the terrain forbids stepping from here to `target`: a cliff
    /// too steep to climb, deep water, or the edge of the map. Only applies
    /// when the player is down on the terrain (not on a bridge or pier).
    fn terrain_blocks(&self, target: Vec3, terrain: &Terrain) -> bool {
        let limit = terrain.half_size - EDGE_MARGIN;
        if target.x.abs() > limit || target.z.abs() > limit {
            return true;
        }
        let here = terrain.height_at(self.pos.x, self.pos.z);
        let there = terrain.height_at(target.x, target.z);
        let on_terrain = self.pos.y - here < 0.3;
        if !on_terrain {
            return false;
        }
        // Deep water: you can't wade further in, but you can always head
        // back towards the shore.
        if there < terrain.water_level - MAX_WADE_DEPTH && there < here {
            return true;
        }
        let run = Vec3::new(target.x - self.pos.x, 0.0, target.z - self.pos.z).length();
        self.grounded && there - here > run * MAX_SLOPE
    }

    /// Moves horizontally by `delta`, stepping up onto low obstacles, or
    /// onto anything up to `reach` high (Mantle).
    fn move_horizontal(&mut self, delta: Vec3, colliders: &[Aabb], terrain: &Terrain, reach: f32) {
        let target = self.pos + delta;
        if self.terrain_blocks(target, terrain) {
            return;
        }
        let bounds = Self::bounds_at(target);
        let blocking: Vec<&Aabb> = colliders.iter().filter(|c| c.intersects(&bounds)).collect();
        if blocking.is_empty() {
            self.pos = target;
            return;
        }

        let climb = if self.grounded { STEP_HEIGHT } else { 0.0 }.max(reach);
        if climb > 0.0 {
            let top = blocking.iter().map(|c| c.max.y).fold(f32::MIN, f32::max);
            if top - self.pos.y <= climb {
                let stepped = Vec3::new(target.x, top, target.z);
                let stepped_bounds = Self::bounds_at(stepped);
                if !colliders.iter().any(|c| c.intersects(&stepped_bounds)) {
                    if top - self.pos.y > STEP_HEIGHT {
                        self.vel_y = self.vel_y.max(0.0); // mantled up
                    }
                    self.pos = stepped;
                }
            }
        }
    }

    /// Moves by `delta` in small steps (so speed can't tunnel through
    /// walls), resolving collisions. Returns the horizontal axes that hit
    /// something (1 = blocked).
    fn travel(&mut self, delta: Vec3, colliders: &[Aabb], terrain: &Terrain, reach: f32) -> Vec3 {
        let steps = (delta.length() / MOVE_STEP).ceil().max(1.0) as usize;
        let step = delta / steps as f32;
        let was_grounded = self.grounded;
        let mut blocked = Vec3::ZERO;
        for _ in 0..steps {
            for axis in [Vec3::X, Vec3::Z] {
                let d = step * axis;
                if d == Vec3::ZERO {
                    continue;
                }
                let before = self.pos;
                self.move_horizontal(d, colliders, terrain, reach);
                let moved = (self.pos - before) * axis;
                if moved.length() < d.length() * 0.5 {
                    blocked += axis;
                }
            }
            // Walking downhill: stay on the ground instead of skipping off
            // it. (Standing on something higher, like a crate, is left
            // alone; the box collision below puts you back on top anyway.)
            let ground = terrain.height_at(self.pos.x, self.pos.z);
            if was_grounded && step.y <= 0.0 && self.pos.y > ground && self.pos.y - ground < STICK_DOWN {
                self.pos.y = ground;
            }

            self.pos.y += step.y;
            self.grounded = false;
            let bounds = Self::bounds_at(self.pos);
            let hits: Vec<&Aabb> = colliders.iter().filter(|c| c.intersects(&bounds)).collect();
            if !hits.is_empty() {
                if step.y <= 0.0 {
                    self.pos.y = hits.iter().map(|c| c.max.y).fold(f32::MIN, f32::max);
                    self.grounded = true;
                } else {
                    self.pos.y = hits.iter().map(|c| c.min.y).fold(f32::MAX, f32::min) - PLAYER_HEIGHT;
                }
                self.vel_y = 0.0;
                self.flight.y = 0.0;
            }
            let ground = terrain.height_at(self.pos.x, self.pos.z);
            if self.pos.y <= ground {
                self.pos.y = ground;
                self.vel_y = 0.0;
                self.grounded = true;
            }
        }
        blocked.min(Vec3::ONE)
    }

    /// `yaw` is the camera's heading; movement is relative to it. Returns
    /// true on the frame a ground slam hits the ground.
    pub fn update(
        &mut self,
        dt: f32,
        controls: &Controls,
        yaw: f32,
        colliders: &[Aabb],
        terrain: &Terrain,
        moves: &Moveset,
    ) -> bool {
        // Walking direction, flattened onto the ground plane.
        let forward = Vec3::new(yaw.cos(), 0.0, yaw.sin());
        let right = Vec3::new(-forward.z, 0.0, forward.x);
        let wish = (forward * controls.forward + right * controls.right).normalize_or_zero();

        if self.grounded {
            self.since_landed += dt;
            self.hover_left = moves.hover_time;
            self.wall_run_left = moves.wall_run;
        } else {
            self.since_landed = 0.0;
            if moves.hover_recharge && !controls.levitate {
                self.hover_left = (self.hover_left + dt * 0.5).min(moves.hover_time);
            }
        }
        if controls.gravity_toggle && moves.low_gravity {
            self.low_gravity = !self.low_gravity;
        }
        self.low_gravity &= moves.low_gravity;
        let gravity = GRAVITY * if self.low_gravity { LOW_GRAVITY } else { 1.0 };

        // True Flight: fly wherever you look while the button is held.
        self.flying = moves.free_flight && controls.levitate && self.hook.is_none();
        if self.flying {
            self.gliding = false;
            self.slamming = false;
            self.fly(dt, controls, yaw, colliders, terrain, moves);
            return self.land();
        }

        let wall = if self.grounded { None } else { self.touching_wall(colliders) };

        // Space in mid-air: a wall jump, or spread/fold the wings.
        let mut jump_used = false;
        if controls.jump && !self.grounded && !self.slamming {
            if let Some(away) = wall.filter(|_| moves.wall_jump && !self.gliding) {
                let along = self.ground_velocity - away * self.ground_velocity.dot(away);
                self.ground_velocity = along + away * WALL_JUMP_PUSH * moves.speed.max(1.0) * moves.wall_jump_power;
                self.vel_y = JUMP_SPEED * moves.jump * moves.wall_jump_power.sqrt();
                jump_used = true;
            } else if self.gliding {
                self.gliding = false;
                self.vel_y = self.flight.y;
                self.ground_velocity = Vec3::new(self.flight.x, 0.0, self.flight.z);
                jump_used = true;
            } else if self.air_jumps_used >= moves.air_jumps && moves.glide && self.hook.is_none() {
                self.gliding = true;
                self.flight = self.ground_velocity + Vec3::Y * self.vel_y;
                self.rocket = 0.0;
                jump_used = true;
            }
        }
        if controls.slam && moves.slam && self.gliding {
            self.gliding = false;
        }
        if self.gliding {
            if controls.boost && self.boosts_used < moves.boosts {
                self.rocket = ROCKET_TIME;
                self.boosts_used += 1;
            }
            self.glide(dt, controls, yaw, colliders, terrain, moves);
            return self.land();
        }

        // Momentum: slides, run-ups and bunny hops build a speed multiplier.
        let mut speed = MOVE_SPEED * moves.speed;
        if controls.sprint {
            speed *= SPRINT_MULTIPLIER;
        }
        if controls.slam && self.grounded && moves.slide && self.ground_velocity.length() > 3.0 {
            self.slide_left = 0.9;
            self.slide_dir = self.ground_velocity.normalize();
            self.momentum = self.momentum.max(1.5);
        }
        let sliding = self.slide_left > 0.0 && self.grounded;
        if self.grounded {
            if sliding {
                self.slide_left -= dt;
            } else if moves.run_up && controls.sprint && wish != Vec3::ZERO {
                if self.momentum < moves.run_up_cap {
                    self.momentum = (self.momentum + 0.35 * dt).min(moves.run_up_cap);
                }
            } else if self.since_landed > BUNNY_HOP_WINDOW && !(moves.momentum_hold && wish != Vec3::ZERO) {
                self.momentum = 1.0 + (self.momentum - 1.0) * (-1.5 * dt).exp();
            }
        }
        self.momentum = self.momentum.clamp(1.0, moves.momentum_cap.max(1.0));

        // Horizontal velocity: chosen freely on the ground; carried through
        // the air (steerable a little, or fully with Air Strafe).
        let target = if sliding { self.slide_dir } else { wish } * speed * self.momentum;
        let mut velocity = if self.hook.is_some() {
            self.ground_velocity
        } else if self.grounded {
            target
        } else if wish == Vec3::ZERO && !moves.air_strafe {
            self.ground_velocity
        } else {
            let carried = self.ground_velocity.length().max(speed * self.momentum);
            let rate = if moves.air_strafe { 14.0 } else { 4.0 };
            (self.ground_velocity + (target - self.ground_velocity) * (dt * rate).min(1.0)).clamp_length_max(carried)
        };

        // Jumps: from the ground (a bunny hop if just landed), or extra
        // ones in mid-air.
        if controls.jump && !jump_used {
            if self.grounded {
                self.vel_y = JUMP_SPEED * moves.jump;
                if moves.bunny_hop && self.since_landed < BUNNY_HOP_WINDOW {
                    self.momentum = (self.momentum * 1.12).min(moves.momentum_cap);
                }
                self.slide_left = 0.0;
            } else if self.air_jumps_used < moves.air_jumps && !self.slamming {
                self.vel_y = JUMP_SPEED * moves.jump;
                self.air_jumps_used += 1;
            }
        }
        // Ground slam: straight down, fast.
        if controls.slam && moves.slam && !self.grounded && !self.slamming && self.hook.is_none() {
            self.slamming = true;
            self.vel_y = -SLAM_SPEED;
            velocity = Vec3::ZERO;
        }

        if let Some(point) = self.hook {
            // Grapple: pulled straight at the hook, gravity ignored; hang
            // there once you arrive.
            self.grounded = false;
            let mut v = velocity + Vec3::Y * self.vel_y;
            let to = point - (self.pos + Vec3::Y * 1.0);
            let distance = to.length();
            if distance < HOOK_ARRIVED {
                v = Vec3::ZERO;
            } else {
                v += to / distance * moves.grapple_pull * dt;
                // Don't orbit: damp sideways drift.
                let along = to / distance * v.dot(to / distance);
                v = along + (v - along) * (-4.0 * dt).exp();
                v = v.clamp_length_max(HOOK_MAX_SPEED.max(moves.grapple_pull * 0.5));
            }
            velocity = Vec3::new(v.x, 0.0, v.z);
            self.vel_y = v.y;
        } else {
            self.vel_y -= gravity * dt;
        }

        // Walls, hover and levitation (in the air).
        self.wall_running = false;
        self.hovering = false;
        if !self.grounded && self.hook.is_none() && !self.slamming {
            if let Some(away) = wall {
                let into = wish.dot(-away);
                if moves.wall_climb && controls.sprint && into > 0.5 {
                    self.vel_y = WALL_CLIMB_SPEED * moves.speed.max(1.0);
                } else if moves.wall_run > 0.0 && controls.sprint && wish != Vec3::ZERO && into < 0.7 && self.wall_run_left > 0.0 {
                    self.vel_y = self.vel_y.max(0.0);
                    self.wall_run_left -= dt;
                    self.wall_running = true;
                } else if moves.wall_slide && self.vel_y < -WALL_SLIDE_SPEED {
                    self.vel_y = -WALL_SLIDE_SPEED;
                }
            }
            if controls.levitate && moves.hover_time > 0.0 && self.hover_left > 0.0 {
                self.vel_y = if moves.levitate { LEVITATE_SPEED } else { 0.0 };
                self.hover_left -= dt;
                self.hovering = true;
            }
        }

        let reach = if wish != Vec3::ZERO { moves.mantle } else { 0.0 };
        let ground_before = terrain.height_at(self.pos.x, self.pos.z);
        let blocked = self.travel(velocity * dt + Vec3::Y * self.vel_y * dt, colliders, terrain, reach);
        velocity *= Vec3::ONE - blocked; // stopped by a wall
        self.ground_velocity = velocity;

        // Downhill Surf: sliding down a slope speeds you up and keeps the
        // slide going.
        if sliding && moves.surf {
            let drop = ground_before - terrain.height_at(self.pos.x, self.pos.z);
            self.momentum = (self.momentum + drop * 0.35).clamp(1.0, moves.momentum_cap);
            if drop > 0.0 {
                self.slide_left = self.slide_left.max(0.15);
            }
        }
        self.land()
    }

    /// Landing resets the air abilities. Returns true if a slam landed.
    fn land(&mut self) -> bool {
        if !self.grounded {
            return false;
        }
        let slammed = self.slamming;
        self.slamming = false;
        self.gliding = false;
        self.rocket = 0.0;
        self.air_jumps_used = 0;
        self.boosts_used = 0;
        slammed
    }

    /// True Flight: move wherever you look, no gravity.
    fn fly(&mut self, dt: f32, controls: &Controls, yaw: f32, colliders: &[Aabb], terrain: &Terrain, moves: &Moveset) {
        let look = Vec3::new(yaw.cos() * controls.pitch.cos(), controls.pitch.sin(), yaw.sin() * controls.pitch.cos());
        let right = Vec3::new(-yaw.sin(), 0.0, yaw.cos());
        let boost = if controls.sprint { 2.0 } else { 1.0 };
        let v = (look * controls.forward + right * controls.right).normalize_or_zero() * FLY_SPEED * moves.speed * moves.fly_speed * boost;
        self.vel_y = v.y;
        let blocked = self.travel(v * dt, colliders, terrain, 0.0);
        self.ground_velocity = Vec3::new(v.x, 0.0, v.z) * (Vec3::ONE - blocked);
        self.vel_y = v.y;
    }

    /// One frame of elytra-style gliding (Minecraft's rules). Looking down
    /// dives, trading height for speed; looking up trades speed for height;
    /// level flight sinks slowly. Velocity turns towards where you look,
    /// and a rocket boost pushes hard along your view for a moment.
    fn glide(&mut self, dt: f32, controls: &Controls, yaw: f32, colliders: &[Aabb], terrain: &Terrain, moves: &Moveset) {
        let k = dt * TICKS; // ticks this frame
        let look = Vec3::new(yaw.cos() * controls.pitch.cos(), controls.pitch.sin(), yaw.sin() * controls.pitch.cos());
        let down_pitch = -controls.pitch; // Minecraft's pitch is positive looking down
        let mut v = self.flight / TICKS;
        let hlook = look.x.hypot(look.z);
        let hvel = v.x.hypot(v.z);
        let lift = down_pitch.cos().powi(2);

        v.y += (-0.08 + lift * 0.06) * k;
        if hlook > 0.0 {
            // Falling: some of the drop turns into forward speed.
            if v.y < 0.0 {
                let a = v.y * -0.1 * lift * k;
                v.y += a;
                v.x += look.x * a / hlook;
                v.z += look.z * a / hlook;
            }
            // Pulling up: forward speed turns into climb.
            if down_pitch < 0.0 {
                let a = hvel * -down_pitch.sin() * 0.04 * k;
                v.y += a * 3.2;
                v.x -= look.x * a / hlook;
                v.z -= look.z * a / hlook;
            }
            // Steer: horizontal velocity swings round to where you look.
            v.x += (look.x / hlook * hvel - v.x) * 0.1 * k;
            v.z += (look.z / hlook * hvel - v.z) * 0.1 * k;
        }
        v *= Vec3::new(0.99f32.powf(k), 0.98f32.powf(k), 0.99f32.powf(k));
        if self.rocket > 0.0 {
            let target = look * (moves.rocket_speed / TICKS);
            v += (look * 0.1 + (target - v) * 0.5) * k.min(1.0);
            self.rocket -= dt;
        }
        self.flight = v * TICKS;

        let blocked = self.travel(self.flight * dt, colliders, terrain, 0.0);
        self.flight *= Vec3::ONE - blocked; // hit a wall: stop that way
        if self.grounded {
            self.vel_y = 0.0;
            self.ground_velocity = Vec3::ZERO;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abilities::{TREES, Tree};

    const DT: f32 = 1.0 / 60.0;
    const WALK_EAST: Controls = Controls {
        forward: 1.0,
        right: 0.0,
        sprint: false,
        jump: false,
        pitch: 0.0,
        slam: false,
        boost: false,
        levitate: false,
        gravity_toggle: false,
    };

    fn with(tree: Tree, level: u32) -> Moveset {
        let mut levels = [0; TREES];
        levels[tree.index()] = level;
        Moveset::from_levels(levels)
    }

    fn ground_box(x: f32, size: Vec3) -> Aabb {
        Aabb::from_center(Vec3::new(x, size.y * 0.5, 0.0), size)
    }

    fn walk(player: &mut Player, controls: &Controls, frames: usize, colliders: &[Aabb]) {
        walk_on(player, controls, frames, colliders, &Terrain::flat(50.0));
    }

    fn walk_on(player: &mut Player, controls: &Controls, frames: usize, colliders: &[Aabb], terrain: &Terrain) {
        for _ in 0..frames {
            player.update(DT, controls, 0.0, colliders, terrain, &Moveset::default()); // yaw 0 = facing +X
        }
    }

    fn run(player: &mut Player, controls: &Controls, frames: usize, colliders: &[Aabb], moves: &Moveset) -> bool {
        let flat = Terrain::flat(80.0);
        (0..frames).any(|_| player.update(DT, controls, 0.0, colliders, &flat, moves))
    }

    #[test]
    fn follows_gentle_slopes_up_and_down() {
        // A 20° ramp up along +X, then back down.
        let hill = Terrain::from_fn(50.0, 1.0, -100.0, |x, _| (10.0 - (x - 10.0).abs()).max(0.0) * 0.36);
        let mut p = Player::new(Vec3::ZERO);
        walk_on(&mut p, &WALK_EAST, 120, &[], &hill); // 10 m: at the top
        assert!((p.pos.y - hill.height_at(p.pos.x, p.pos.z)).abs() < 1e-3);
        assert!(p.pos.x > 9.0);
        walk_on(&mut p, &WALK_EAST, 60, &[], &hill); // walking down: stays on the ground
        assert!((p.pos.y - hill.height_at(p.pos.x, p.pos.z)).abs() < 1e-3, "floating at {}", p.pos.y);
    }

    #[test]
    fn cliffs_and_deep_water_block() {
        let cliff = Terrain::from_fn(50.0, 1.0, -100.0, |x, _| if x > 5.0 { (x - 5.0) * 3.0 } else { 0.0 });
        let mut p = Player::new(Vec3::ZERO);
        walk_on(&mut p, &WALK_EAST, 120, &[], &cliff);
        assert!(p.pos.x < 6.0 && p.pos.y < 1.0, "climbed a cliff: {:?}", p.pos);

        let lake = Terrain::from_fn(50.0, 1.0, 0.0, |x, _| if x > 5.0 { -2.0 } else { 0.5 });
        let mut p = Player::new(Vec3::new(0.0, 0.5, 0.0));
        walk_on(&mut p, &WALK_EAST, 120, &[], &lake);
        assert!(p.pos.x < 6.0, "walked into deep water: {:?}", p.pos);
    }

    #[test]
    fn wall_blocks_movement() {
        let wall = [ground_box(3.0, Vec3::new(1.0, 3.0, 10.0))];
        let mut p = Player::new(Vec3::ZERO);
        walk(&mut p, &WALK_EAST, 120, &wall);
        assert!(p.pos.x <= 2.5 - PLAYER_RADIUS + 1e-4, "walked into the wall: {}", p.pos.x);
        assert!(p.pos.x > 2.0, "should reach the wall");
    }

    #[test]
    fn steps_up_low_ledges_but_not_high_ones() {
        let step = [ground_box(2.0, Vec3::new(2.0, 0.4, 4.0))];
        let mut p = Player::new(Vec3::ZERO);
        walk(&mut p, &WALK_EAST, 30, &step);
        assert!((p.pos.y - 0.4).abs() < 1e-4, "should be on the step, y = {}", p.pos.y);

        let ledge = [ground_box(2.0, Vec3::new(2.0, 1.0, 4.0))];
        let mut p = Player::new(Vec3::ZERO);
        walk(&mut p, &WALK_EAST, 60, &ledge);
        assert_eq!(p.pos.y, 0.0);
        assert!(p.pos.x < 1.0);
    }

    #[test]
    fn jump_lands_on_box() {
        // A 1 m crate is too tall to step onto, so it takes a jump.
        let crate_box = [ground_box(1.5, Vec3::new(1.0, 1.0, 1.0))];
        let mut p = Player::new(Vec3::new(0.69, 0.0, 0.0)); // touching its side
        let jump = Controls { jump: true, ..WALK_EAST };
        walk(&mut p, &jump, 1, &crate_box);
        walk(&mut p, &WALK_EAST, 12, &crate_box);
        let idle = Controls { forward: 0.0, ..WALK_EAST };
        walk(&mut p, &idle, 60, &crate_box);
        assert!((p.pos.y - 1.0).abs() < 1e-4, "should stand on the crate, y = {}", p.pos.y);
    }

    fn on_flat(player: &mut Player, controls: &Controls, frames: usize, moves: &Moveset) -> bool {
        run(player, controls, frames, &[], moves)
    }

    fn apex(moves: &Moveset, double: bool) -> f32 {
        let mut p = Player::new(Vec3::ZERO);
        let jump = Controls { jump: true, ..Default::default() };
        let idle = Controls::default();
        on_flat(&mut p, &jump, 1, moves);
        let mut top: f32 = 0.0;
        for i in 0..120 {
            let c = if double && i == 20 { &jump } else { &idle };
            on_flat(&mut p, c, 1, moves);
            top = top.max(p.pos.y);
        }
        top
    }

    #[test]
    fn high_jump_and_double_jump_go_higher() {
        let base = apex(&Moveset::default(), false);
        let high = with(Tree::Air, 1);
        assert!(apex(&high, false) > base * 1.5, "high jump");
        let double = with(Tree::Air, 2);
        assert!(apex(&double, true) > apex(&double, false) + 0.5, "double jump");
        // Without the ability a mid-air jump does nothing.
        assert!((apex(&Moveset::default(), true) - base).abs() < 1e-3);
    }

    #[test]
    fn gliding_falls_slowly_and_slam_lands_hard() {
        let glide = with(Tree::Air, 4);
        let spread = Controls { jump: true, ..Default::default() };
        let level = Controls::default();
        let mut p = Player::new(Vec3::new(0.0, 40.0, 0.0));
        p.grounded = false;
        on_flat(&mut p, &spread, 1, &glide); // the double jump comes first
        assert!(!p.gliding);
        on_flat(&mut p, &spread, 1, &glide);
        assert!(p.gliding);
        on_flat(&mut p, &level, 120, &glide); // 2 s, looking level
        assert!(p.gliding && p.pos.y > 25.0, "level flight should sink slowly, y = {}", p.pos.y);
        // From a standstill, level flight only creeps forward (dive for speed).
        assert!(p.pos.x > 1.0, "and glide forwards, x = {}", p.pos.x);

        // Diving builds speed; pulling up trades it for lift (and, as in
        // Minecraft, stalls if you're too slow: rockets are for climbing).
        let dive = Controls { pitch: -1.2, ..Default::default() };
        p.pos.y = 45.0; // room for a long dive
        on_flat(&mut p, &dive, 90, &glide);
        let fast = p.flight.length();
        assert!(fast > 15.0, "dive speed {fast}");
        let falling = p.flight.y;
        let climb = Controls { pitch: 0.6, ..Default::default() };
        on_flat(&mut p, &climb, 30, &glide);
        assert!(p.flight.y > falling + 5.0, "pulling up should check the fall: {} from {falling}", p.flight.y);
        assert!(p.flight.length() < fast, "pulling up costs speed");

        // A rocket speeds you up along your view.
        let mut p = Player::new(Vec3::new(0.0, 40.0, 0.0));
        p.grounded = false;
        on_flat(&mut p, &spread, 2, &glide);
        on_flat(&mut p, &Controls { boost: true, ..Default::default() }, 1, &glide);
        on_flat(&mut p, &level, 50, &glide);
        assert!(p.flight.x > 20.0 && p.boosts_used == 1, "rocket: {:?}", p.flight);

        let mut p = Player::new(Vec3::new(0.0, 30.0, 0.0));
        let slam = Controls { slam: true, ..Default::default() };
        let landed = on_flat(&mut p, &slam, 120, &glide);
        assert!(landed && p.pos.y == 0.0, "slam should hit the ground");
    }

    #[test]
    fn blink_stops_short_of_walls() {
        let wall = [ground_box(5.0, Vec3::new(1.0, 3.0, 10.0))];
        let flat = Terrain::flat(50.0);
        let mut p = Player::new(Vec3::ZERO);
        assert!(p.blink(Vec3::X, 9.0, &wall, &flat));
        assert!(p.pos.x > 3.5 && p.pos.x <= 4.5 - PLAYER_RADIUS + 1e-3, "x = {}", p.pos.x);
    }

    /// A tall wall along X at z = 1 (you stand south of it, at z = 0).
    fn long_wall() -> [Aabb; 1] {
        [Aabb::from_center(Vec3::new(0.0, 5.0, 1.0 + PLAYER_RADIUS + 0.25), Vec3::new(80.0, 10.0, 0.5))]
    }

    #[test]
    fn wall_slide_jump_and_run() {
        let wall = long_wall();
        // Slide: falling beside the wall is slow.
        let slide = with(Tree::Parkour, 1);
        let mut p = Player::new(Vec3::new(0.0, 8.0, 0.96));
        p.grounded = false;
        run(&mut p, &Controls::default(), 30, &wall, &slide);
        assert!(p.vel_y >= -WALL_SLIDE_SPEED - 1e-3, "sliding at {}", p.vel_y);

        // Jump: kicks away from the wall (towards -Z) and up.
        let jump = with(Tree::Parkour, 2);
        let mut p = Player::new(Vec3::new(0.0, 8.0, 0.96));
        p.grounded = false;
        run(&mut p, &Controls { jump: true, ..Default::default() }, 1, &wall, &jump);
        assert!(p.vel_y > 5.0 && p.ground_velocity.z < -5.0, "{:?} {}", p.ground_velocity, p.vel_y);

        // Run: sprinting along it in mid-air, you don't fall.
        let wall_run = with(Tree::Parkour, 3);
        let mut p = Player::new(Vec3::new(0.0, 8.0, 0.96));
        p.grounded = false;
        p.refill(&wall_run); // as if just jumped from the ground
        let along = Controls { forward: 1.0, sprint: true, ..Default::default() };
        run(&mut p, &along, 60, &wall, &wall_run);
        assert!(p.pos.y > 7.5 && p.pos.x > 5.0, "{:?}", p.pos);
        run(&mut p, &along, 60, &wall, &wall_run); // 1.5 s limit runs out
        assert!(p.pos.y < 7.0, "wall run should end: {:?}", p.pos);
    }

    #[test]
    fn mantle_climbs_high_ledges() {
        let ledge = [ground_box(2.0, Vec3::new(2.0, 2.2, 4.0))];
        let mut p = Player::new(Vec3::ZERO);
        run(&mut p, &WALK_EAST, 25, &ledge, &with(Tree::Parkour, 4)); // 2 m: on top
        assert!((p.pos.y - 2.2).abs() < 1e-3, "should be up on the ledge: {:?}", p.pos);
    }

    #[test]
    fn spider_climbs_walls() {
        let wall = [ground_box(3.0, Vec3::new(1.0, 12.0, 10.0))];
        let mut p = Player::new(Vec3::ZERO);
        let into = Controls { sprint: true, ..WALK_EAST };
        run(&mut p, &Controls { jump: true, ..into }, 1, &wall, &with(Tree::Parkour, 5));
        run(&mut p, &into, 60, &wall, &with(Tree::Parkour, 5));
        assert!(p.pos.y > 5.0, "should climb the wall: {:?}", p.pos);
    }

    #[test]
    fn slides_and_bunny_hops_build_speed() {
        let moves = with(Tree::Momentum, 2);
        let mut p = Player::new(Vec3::new(-60.0, 0.0, 0.0));
        let sprint = Controls { sprint: true, ..WALK_EAST };
        run(&mut p, &sprint, 30, &[], &moves);
        let base = p.ground_velocity.length();
        run(&mut p, &Controls { slam: true, ..sprint }, 1, &[], &moves);
        assert!(p.ground_velocity.length() > base * 1.4, "slide");
        // Hop every landing.
        for _ in 0..8 {
            run(&mut p, &Controls { jump: true, ..sprint }, 1, &[], &moves);
            while !p.grounded {
                run(&mut p, &sprint, 1, &[], &moves);
            }
        }
        assert!(p.momentum() > 2.5, "bunny hops should build momentum: {}", p.momentum());
    }

    #[test]
    fn hover_levitate_and_fly() {
        let mut p = Player::new(Vec3::new(0.0, 10.0, 0.0));
        p.grounded = false;
        let hold = Controls { levitate: true, ..Default::default() };
        p.refill(&with(Tree::Gravity, 2)); // as if just jumped from the ground
        run(&mut p, &hold, 60, &[], &with(Tree::Gravity, 2));
        assert!(p.pos.y > 9.5, "hovering: {}", p.pos.y);

        let mut p = Player::new(Vec3::ZERO);
        let moves = with(Tree::Gravity, 3);
        run(&mut p, &Controls { jump: true, ..hold }, 1, &[], &moves);
        run(&mut p, &hold, 60, &[], &moves);
        assert!(p.pos.y > 6.0, "levitating: {}", p.pos.y);

        let mut p = Player::new(Vec3::ZERO);
        let up = Controls { levitate: true, forward: 1.0, pitch: 0.8, ..Default::default() };
        run(&mut p, &up, 60, &[], &with(Tree::Gravity, 5));
        assert!(p.pos.y > 8.0 && p.pos.x > 8.0, "flying: {:?}", p.pos);
    }

    #[test]
    fn low_gravity_jumps_higher() {
        let moves = with(Tree::Gravity, 1);
        let mut p = Player::new(Vec3::ZERO);
        run(&mut p, &Controls { gravity_toggle: true, ..Default::default() }, 1, &[], &moves);
        assert!(p.low_gravity());
        assert!(apex(&moves, false) < 2.0);
        run(&mut p, &Controls { jump: true, ..Default::default() }, 1, &[], &moves);
        let mut top: f32 = 0.0;
        for _ in 0..120 {
            run(&mut p, &Controls::default(), 1, &[], &moves);
            top = top.max(p.pos.y);
        }
        assert!(top > 3.0, "moon jump: {top}");
    }

    #[test]
    fn grapple_pulls_to_the_hook_and_slingshot_flings() {
        let moves = with(Tree::Gadgets, 5);
        let mut p = Player::new(Vec3::ZERO);
        let hook = Vec3::new(20.0, 10.0, 0.0);
        p.set_hook(hook);
        run(&mut p, &Controls::default(), 90, &[], &moves);
        assert!((p.pos + Vec3::Y).distance(hook) < 2.0, "should reach the hook: {:?}", p.pos);
        // Hanging there...
        let before = p.pos;
        run(&mut p, &Controls::default(), 30, &[], &moves);
        assert!(p.pos.distance(before) < 0.5);
        // ...until let go, with a fling.
        p.release_hook(true, &moves);
        assert!(p.vel_y > 4.0);
    }
}
