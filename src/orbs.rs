//! Skill crystals: glowing, spinning crystals hidden around the valley.
//! Walking into one collects it for a skill point.

use glam::{Mat4, Quat, Vec2, Vec3, Vec4};

use crate::mesh::MeshData;
use crate::model::MaterialData;
use crate::renderer::Renderer;

/// How close (meters, to the player's chest) counts as touching a crystal.
const PICKUP_RADIUS: f32 = 1.4;

pub struct Orbs {
    spots: Vec<Vec3>,
    handles: Vec<usize>,
    collected: Vec<bool>,
    time: f32,
}

/// A long, faceted crystal (two pyramids base to base).
fn crystal_mesh() -> MeshData {
    let mut m = MeshData::default();
    let (top, bottom) = (Vec3::new(0.0, 0.55, 0.0), Vec3::new(0.0, -0.45, 0.0));
    let sides = 6;
    let ring = |i: usize| {
        let a = i as f32 / sides as f32 * std::f32::consts::TAU;
        Vec3::new(a.cos() * 0.25, 0.05, a.sin() * 0.25)
    };
    for i in 0..sides {
        let (a, b) = (ring(i), ring(i + 1));
        let uv = [Vec2::ZERO; 3];
        let out = (a + b) * 0.5;
        m.push_tri([top, a, b], uv, Vec4::ONE, out + Vec3::Y * 0.3);
        m.push_tri([bottom, a, b], uv, Vec4::ONE, out - Vec3::Y * 0.3);
    }
    m
}

impl Orbs {
    /// `collected` lists crystals found in earlier sessions.
    pub fn new(renderer: &mut Renderer, spots: Vec<Vec3>, collected: &[usize]) -> Self {
        let mesh = crystal_mesh();
        // Brighter than white: reads as glowing after tone mapping.
        let material = MaterialData {
            base_color: Vec4::new(0.7, 1.6, 2.4, 1.0),
            specular: 1.0,
            shininess: 80.0,
            ground_occlusion: 0.0,
            ..Default::default()
        };
        let handles = spots
            .iter()
            .map(|_| renderer.add_dynamic_mesh(&mesh, &material))
            .collect();
        let collected = (0..spots.len()).map(|i| collected.contains(&i)).collect();
        Self {
            spots,
            handles,
            collected,
            time: 0.0,
        }
    }

    pub fn total(&self) -> usize {
        self.spots.len()
    }

    /// Spins and bobs the crystals; returns the index of one touched by a
    /// player whose chest is at `chest`.
    pub fn update(&mut self, renderer: &mut Renderer, dt: f32, chest: Vec3) -> Option<usize> {
        self.time += dt;
        let mut touched = None;
        for (i, &spot) in self.spots.iter().enumerate() {
            if self.collected[i] {
                renderer.set_transform(self.handles[i], Mat4::IDENTITY, false);
                continue;
            }
            let bob = (self.time * 2.0 + i as f32).sin() * 0.15;
            let spin = Quat::from_rotation_y(self.time * 1.5 + i as f32);
            let pos = spot + Vec3::Y * bob;
            renderer.set_transform(self.handles[i], Mat4::from_rotation_translation(spin, pos), true);
            if touched.is_none() && pos.distance(chest) < PICKUP_RADIUS {
                self.collected[i] = true;
                touched = Some(i);
            }
        }
        touched
    }
}
