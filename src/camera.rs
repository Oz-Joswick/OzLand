//! First-person perspective camera producing view and projection matrices.

use glam::{Mat4, Vec2, Vec3, camera};

pub struct Camera {
    pub position: Vec3,
    /// Rotation around +Y in radians; 0 looks along +X, -PI/2 along -Z.
    pub yaw: f32,
    /// Up/down rotation in radians, clamped just short of straight up/down.
    pub pitch: f32,
    pub fov_y: f32,
    pub aspect: f32,
    pub near: f32,
}

const PITCH_LIMIT: f32 = 1.55;

impl Camera {
    pub fn new(position: Vec3, yaw: f32, aspect: f32) -> Self {
        Self {
            position,
            yaw,
            pitch: 0.0,
            fov_y: 70f32.to_radians(),
            aspect,
            near: 0.05,
        }
    }

    /// Applies a mouse movement in pixels.
    pub fn rotate(&mut self, mouse_delta: Vec2, sensitivity: f32) {
        self.yaw += mouse_delta.x * sensitivity;
        self.pitch = (self.pitch - mouse_delta.y * sensitivity).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    }

    pub fn forward(&self) -> Vec3 {
        Vec3::new(
            self.yaw.cos() * self.pitch.cos(),
            self.pitch.sin(),
            self.yaw.sin() * self.pitch.cos(),
        )
    }

    /// World -> camera space (right-handed, looking down -Z).
    pub fn view(&self) -> Mat4 {
        camera::rh::view::look_to_mat4(self.position, self.forward(), Vec3::Y)
    }

    /// Camera -> clip space. wgpu's clip space has depth 0..1 and Y up, the
    /// same convention as Direct3D. Reverse-Z with an infinite far plane:
    /// depth is 1 at the near plane and approaches 0 at infinity, which
    /// keeps floating-point depth precise across a big landscape.
    pub fn projection(&self) -> Mat4 {
        camera::rh::proj::directx::perspective_infinite_reverse(self.fov_y, self.aspect, self.near)
    }

    pub fn view_projection(&self) -> Mat4 {
        self.projection() * self.view()
    }
}
