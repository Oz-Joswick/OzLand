//! Axis-aligned bounding boxes used for collision (and model bounds).

use glam::{Mat4, Vec3};

#[derive(Clone, Copy, Debug)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    pub fn from_center(center: Vec3, size: Vec3) -> Self {
        let half = size * 0.5;
        Self {
            min: center - half,
            max: center + half,
        }
    }

    /// Strict overlap test: boxes that only touch (e.g. standing exactly on
    /// top of a crate) do not count as intersecting.
    pub fn intersects(&self, other: &Aabb) -> bool {
        self.min.x < other.max.x
            && self.max.x > other.min.x
            && self.min.y < other.max.y
            && self.max.y > other.min.y
            && self.min.z < other.max.z
            && self.max.z > other.min.z
    }

    pub fn size(&self) -> Vec3 {
        self.max - self.min
    }

    /// The axis-aligned box that encloses this box after `transform`.
    pub fn transformed(&self, transform: Mat4) -> Aabb {
        let mut out = Aabb {
            min: Vec3::splat(f32::MAX),
            max: Vec3::splat(f32::MIN),
        };
        for i in 0..8 {
            let corner = Vec3::new(
                if i & 1 == 0 { self.min.x } else { self.max.x },
                if i & 2 == 0 { self.min.y } else { self.max.y },
                if i & 4 == 0 { self.min.z } else { self.max.z },
            );
            let p = transform.transform_point3(corner);
            out.min = out.min.min(p);
            out.max = out.max.max(p);
        }
        out
    }

    pub fn union(&self, other: &Aabb) -> Aabb {
        Aabb {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }
}
