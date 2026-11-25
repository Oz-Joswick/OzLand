//! The player's body: an animated character that follows the player.
//!
//! The model is RobotExpressive by Tomás Laulhé (CC0), from the three.js
//! examples, with its own Idle / Walking / Running / Jump animations. The
//! animation is chosen from how the player is actually moving and
//! cross-fades between clips. In first person it isn't drawn (you can't see
//! yourself), but it keeps animating, ready for third person (V). Any glTF character with clips of those names can
//! replace it (see `MODEL`).

use std::rc::Rc;

use glam::{Mat4, Quat, Vec2, Vec3, Vec4};

use crate::animation::AnimatedModel;
use crate::model;
use crate::renderer::Renderer;

const MODEL: &str = "characters/robot/RobotExpressive.glb";
const HEIGHT: f32 = 1.8;
/// How long a change of animation takes to blend in.
const FADE: f32 = 0.2;
/// Speeds (m/s) the walk and run clips were animated at, so playback can
/// match the player's real speed and the feet don't slide.
const WALK_CLIP_SPEED: f32 = 3.0;
const RUN_CLIP_SPEED: f32 = 7.0;

pub struct Body {
    model: Option<Rc<AnimatedModel>>,
    /// Renderer handles, one per model part.
    handles: Vec<usize>,
    /// Scale and offset that put the feet at the origin and make it HEIGHT tall.
    fit: Mat4,
    current: Option<usize>,
    current_time: f32,
    previous: Option<(usize, f32)>,
    fade: f32,
    last_feet: Option<Vec3>,
    /// Tilt towards the direction of flight while gliding (eased).
    lean: Quat,
    /// Per-part colour multipliers (the player's colour on the painted parts).
    tints: Vec<Vec4>,
}

impl Body {
    /// Loads the character model (shared by the player and clones). If it
    /// is missing the error is logged and bodies are simply not drawn.
    pub fn load_model() -> Option<Rc<AnimatedModel>> {
        match AnimatedModel::load(&model::asset_root().join(MODEL)) {
            Ok(m) => Some(Rc::new(m)),
            Err(e) => {
                log::error!("character: {e:#}");
                None
            }
        }
    }

    /// A body using model, added to the renderer (hidden until updated).
    pub fn new(renderer: &mut Renderer, model: Option<Rc<AnimatedModel>>) -> Self {
        let mut body = Self {
            model: None,
            handles: Vec::new(),
            fit: Mat4::IDENTITY,
            current: None,
            current_time: 0.0,
            previous: None,
            fade: 1.0,
            last_feet: None,
            lean: Quat::IDENTITY,
            tints: Vec::new(),
        };
        let Some(model) = model else {
            return body;
        };
        let (lo, hi) = model.rest_bounds();
        let scale = HEIGHT / (hi.y - lo.y).max(0.01);
        let center = (lo + hi) * 0.5;
        body.fit = Mat4::from_scale(Vec3::splat(scale)) * Mat4::from_translation(-Vec3::new(center.x, lo.y, center.z));

        let rest = model.pose(None, 0.0, None);
        for part in &model.parts {
            let mut mesh = crate::mesh::MeshData {
                vertices: model.skin(part, &rest),
                indices: part.indices.clone(),
            };
            mesh.compute_tangents();
            body.handles.push(renderer.add_dynamic_mesh(&mesh, &part.material));
        }
        body.current = model.clip("Idle");
        body.model = Some(model);
        body.hide(renderer);
        body
    }

    /// Repaints the body's coloured panels (the robot's gold parts) in
    /// `color` (linear RGB); grey and black details keep their colour.
    pub fn set_color(&mut self, color: Vec3) {
        let Some(model) = &self.model else { return };
        self.tints = model
            .parts
            .iter()
            .map(|part| {
                let base = part.material.base_color.truncate();
                let painted = base.max_element() - base.min_element() > 0.2;
                if painted {
                    (color / base.max(Vec3::splat(0.01))).extend(1.0)
                } else {
                    Vec4::ONE
                }
            })
            .collect();
    }

    /// Stops drawing this body.
    pub fn hide(&self, renderer: &mut Renderer) {
        for &h in &self.handles {
            renderer.set_transform(h, Mat4::IDENTITY, false);
        }
    }

    /// Forgets the last position (so a teleport isn't read as running).
    pub fn reset_motion(&mut self) {
        self.last_feet = None;
    }

    /// Animates and places the body at the player's feet, facing `yaw`.
    /// `flight` is the direction of travel while gliding: the body lies
    /// along it, head first, like an elytra flyer.
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        renderer: &mut Renderer,
        dt: f32,
        feet: Vec3,
        yaw: f32,
        grounded: bool,
        flight: Option<Vec3>,
        visible: bool,
    ) {
        let Some(model) = &self.model else { return };

        // How fast the player is really moving (from the position change).
        let speed = match self.last_feet {
            Some(last) if dt > 0.0 => Vec2::new(feet.x - last.x, feet.z - last.z).length() / dt,
            _ => 0.0,
        };
        self.last_feet = Some(feet);

        let wanted = if !grounded {
            model.clip("Jump")
        } else if speed > 6.0 {
            model.clip("Running")
        } else if speed > 0.4 {
            model.clip("Walking")
        } else {
            model.clip("Idle")
        };
        if wanted != self.current {
            if let Some(c) = self.current {
                self.previous = Some((c, self.current_time));
            }
            self.current = wanted;
            self.current_time = 0.0;
            self.fade = 0.0;
        }

        // Advance time: walk/run in step with the real speed, jump plays once.
        let rate = match self.current {
            c if c == model.clip("Walking") => (speed / WALK_CLIP_SPEED).clamp(0.5, 1.8),
            c if c == model.clip("Running") => (speed / RUN_CLIP_SPEED).clamp(0.6, 1.6),
            _ => 1.0,
        };
        self.current_time += dt * rate;
        if let Some(c) = self.current {
            let duration = model.clips[c].duration.max(0.01);
            self.current_time = if Some(c) == model.clip("Jump") {
                self.current_time.min(duration)
            } else {
                self.current_time % duration
            };
        }
        self.fade = (self.fade + dt / FADE).min(1.0);

        // Pose: the old clip fading into the new one.
        let pose = match (self.previous, self.current) {
            (Some((prev, prev_time)), Some(cur)) if self.fade < 1.0 => {
                model.pose(Some(prev), prev_time, Some((cur, self.current_time, self.fade)))
            }
            _ => model.pose(self.current, self.current_time, None),
        };

        // glTF characters face +Z; the camera's forward is (cos yaw, 0, sin yaw).
        let target = flight.map_or(Quat::IDENTITY, |d| Quat::from_rotation_arc(Vec3::Y, d));
        self.lean = self.lean.slerp(target, 1.0 - (-8.0 * dt).exp());
        // Lean around the middle of the body, not the feet.
        let middle = Vec3::Y * (HEIGHT * 0.5);
        let place = Mat4::from_translation(feet + middle)
            * Mat4::from_quat(self.lean)
            * Mat4::from_translation(-middle)
            * Mat4::from_quat(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2 - yaw))
            * self.fit;
        for (i, part) in model.parts.iter().enumerate() {
            renderer.set_vertices(self.handles[i], &model.skin(part, &pose));
            let tint = self.tints.get(i).copied().unwrap_or(Vec4::ONE);
            renderer.set_transform_tinted(self.handles[i], place, tint, visible);
        }
    }
}
