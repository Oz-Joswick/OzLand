//! Scene building blocks: placing boxes and model files on the terrain,
//! plus reusable structures (houses, roofs, fences, benches). The world
//! itself is laid out in `map.rs`.

use std::collections::HashMap;
use std::path::PathBuf;

use glam::{Mat4, Quat, Vec2, Vec3, Vec4};

use crate::collision::Aabb;
use crate::mesh::{BoxUv, MeshData};
use crate::model::{self, MaterialData, Model, ModelPart, TextureData};
use crate::renderer::{GrassField, Instance, Lighting, TerrainData};
use crate::terrain::Terrain;
use crate::textures::{Surface, SurfaceTextures};

/// How a placed model blocks the player.
#[derive(Clone, Copy)]
pub enum Collider {
    /// Walk-through (decoration).
    None,
    /// The model's whole bounding box, after placement.
    Bounds,
    /// A box of this size (world units) standing on the ground, centered on
    /// the model's own origin (usually its trunk/post/feet). Useful when the
    /// bounding box is much bigger than the solid part.
    Box(Vec3),
}

/// Where and how to put a model in the world. The model is moved so the
/// bottom-center of its bounding box sits at `position`, so files work no
/// matter where their origin is. By default it stands on the terrain.
#[derive(Clone, Copy)]
pub struct Placement {
    pub position: Vec2,
    /// Height of the model's base; `None` = on the terrain (slightly sunk
    /// in, so it never floats on a slope).
    pub y: Option<f32>,
    /// Rotation around the vertical axis, in degrees.
    pub yaw_degrees: f32,
    pub scale: f32,
    /// If set, scale the model so it is this tall (then `scale` is applied on top).
    pub height: Option<f32>,
    pub collider: Collider,
    /// Linear RGBA multiplied with the model's colors (for variety).
    pub tint: Vec4,
}

impl Placement {
    /// On the ground at (x, z), unrotated, original size, solid bounds.
    pub fn at(x: f32, z: f32) -> Self {
        Self {
            position: Vec2::new(x, z),
            y: None,
            yaw_degrees: 0.0,
            scale: 1.0,
            height: None,
            collider: Collider::Bounds,
            tint: Vec4::ONE,
        }
    }
    pub fn at_height(mut self, y: f32) -> Self {
        self.y = Some(y);
        self
    }
    pub fn yaw(mut self, degrees: f32) -> Self {
        self.yaw_degrees = degrees;
        self
    }
    pub fn scale(mut self, scale: f32) -> Self {
        self.scale = scale;
        self
    }
    pub fn height(mut self, height: f32) -> Self {
        self.height = Some(height);
        self
    }
    pub fn collider(mut self, collider: Collider) -> Self {
        self.collider = collider;
        self
    }
    pub fn tint(mut self, tint: Vec4) -> Self {
        self.tint = tint;
        self
    }
}

/// A loaded model and every place it appears.
pub struct PlacedModel {
    pub model: Model,
    pub instances: Vec<Instance>,
}

/// Which wall of a building something is on.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// -Z
    North,
    /// +Z
    South,
    /// +X
    East,
    /// -X
    West,
}

impl Side {
    pub fn normal(self) -> Vec2 {
        match self {
            Side::North => Vec2::new(0.0, -1.0),
            Side::South => Vec2::new(0.0, 1.0),
            Side::East => Vec2::new(1.0, 0.0),
            Side::West => Vec2::new(-1.0, 0.0),
        }
    }
}

/// A simple house: four walls (one with a door), a wooden floor and a
/// pitched roof whose ridge runs parallel to the door wall.
pub struct House {
    pub center: Vec2,
    /// Outer footprint (X, Z).
    pub size: Vec2,
    pub door: Side,
    pub wall: Surface,
    pub wall_tint: Vec4,
    pub wall_height: f32,
    pub roof_rise: f32,
    pub roof_tint: Vec4,
}

pub struct Scene {
    /// Where the skill crystals float.
    pub crystals: Vec<Vec3>,
    /// Geometry built in code; drawn once with an identity transform.
    pub static_parts: Vec<ModelPart>,
    /// 3D grass over the ground, if any.
    pub grass: Option<GrassField>,
    pub models: Vec<PlacedModel>,
    pub colliders: Vec<Aabb>,
    pub terrain: Terrain,
    /// Ground layers in splat order: grass, dirt, rock, sand, paving.
    pub ground_layers: [SurfaceTextures; 5],
    pub ground_splat: TextureData,
    pub spawn: Vec3,
    pub spawn_yaw: f32,
    pub lighting: Lighting,
}

impl Scene {
    /// What the renderer needs to draw the terrain (`mesh` is `terrain.mesh()`).
    pub fn terrain_data<'a>(&'a self, mesh: &'a MeshData) -> TerrainData<'a> {
        let l = &self.ground_layers;
        TerrainData {
            mesh,
            heights: &self.terrain.heights,
            res: self.terrain.res as u32,
            half_size: self.terrain.half_size,
            cell: self.terrain.cell,
            water_level: self.terrain.water_level,
            layers: [&l[0], &l[1], &l[2], &l[3], &l[4]],
            tile_meters: [4.0, 3.0, 8.0, 4.0, 3.0],
            specular: [0.04, 0.06, 0.15, 0.08, 0.2],
            splat: &self.ground_splat,
            macro_variation: 0.35,
        }
    }
}

/// Shading settings for each procedural surface.
pub fn surface_material(surface: Surface, tex: &SurfaceTextures) -> MaterialData {
    let base = MaterialData {
        base_color_texture: Some(tex.albedo.clone()),
        normal_texture: Some(tex.normal.clone()),
        ..Default::default()
    };
    // Parallax depth is in UV units: e.g. 0.025 on a texture repeating every
    // 2 m is 5 cm of apparent relief.
    let (specular, shininess, macro_variation, ground_occlusion, parallax_depth) = match surface {
        Surface::Grass => (0.04, 8.0, 0.35, 0.0, 0.0),
        Surface::StoneWall => (0.12, 16.0, 0.15, 0.4, 0.025),
        Surface::Brick => (0.10, 16.0, 0.1, 0.4, 0.008),
        Surface::Tiles => (0.25, 40.0, 0.1, 0.3, 0.012),
        Surface::Crate => (0.15, 20.0, 0.0, 0.4, 0.04),
        Surface::Planks => (0.15, 24.0, 0.05, 0.2, 0.006),
        Surface::Shingles => (0.12, 24.0, 0.05, 0.0, 0.02),
        Surface::Cobble => (0.20, 24.0, 0.2, 0.0, 0.015),
        Surface::Sandstone => (0.12, 16.0, 0.1, 0.35, 0.008),
        Surface::Dirt => (0.06, 12.0, 0.15, 0.2, 0.01),
        Surface::Rock => (0.15, 20.0, 0.15, 0.3, 0.02),
        Surface::Sand => (0.08, 12.0, 0.1, 0.0, 0.006),
    };
    MaterialData {
        specular,
        shininess,
        macro_variation,
        ground_occlusion,
        parallax_depth,
        ..base
    }
}

/// How far boxes reach down into the ground, so they never float on
/// uneven terrain.
const SINK: f32 = 0.35;

pub struct SceneBuilder {
    pub terrain: Terrain,
    assets: PathBuf,
    surfaces: HashMap<Surface, MeshData>,
    pub models: Vec<PlacedModel>,
    loaded: HashMap<String, Option<usize>>,
    pub colliders: Vec<Aabb>,
}

impl SceneBuilder {
    pub fn new(terrain: Terrain) -> Self {
        Self {
            terrain,
            assets: model::asset_root(),
            surfaces: HashMap::new(),
            models: Vec::new(),
            loaded: HashMap::new(),
            colliders: Vec::new(),
        }
    }

    pub fn ground(&self, x: f32, z: f32) -> f32 {
        self.terrain.height_at(x, z)
    }

    /// Lowest terrain height under a rectangular footprint.
    pub fn ground_under(&self, center: Vec2, half: Vec2) -> f32 {
        let mut lowest = f32::MAX;
        for dz in [-1.0, 0.0, 1.0] {
            for dx in [-1.0, 0.0, 1.0] {
                lowest = lowest.min(self.ground(center.x + dx * half.x, center.y + dz * half.y));
            }
        }
        lowest
    }

    fn mesh(&mut self, surface: Surface) -> &mut MeshData {
        self.surfaces.entry(surface).or_default()
    }

    /// A box that is drawn but not solid.
    pub fn visual(&mut self, surface: Surface, center: Vec3, size: Vec3, tint: Vec4, uv: BoxUv) {
        self.mesh(surface).push_box(center, size, tint, uv);
    }

    /// A box that is drawn and solid.
    pub fn solid(&mut self, surface: Surface, center: Vec3, size: Vec3, tint: Vec4, uv: BoxUv) {
        self.visual(surface, center, size, tint, uv);
        self.colliders.push(Aabb::from_center(center, size));
    }

    /// A round column (or, with a sideways `axis`, a log or fallen pillar).
    /// Solid ones get a box collider around them.
    #[allow(clippy::too_many_arguments)]
    pub fn cylinder(&mut self, surface: Surface, base: Vec3, axis: Vec3, radius: f32, length: f32, tint: Vec4, solid: bool) {
        let segments = (radius * 24.0).clamp(10.0, 32.0) as u32;
        self.mesh(surface)
            .push_cylinder(base, axis, radius, length, segments, tint, 2.0);
        if solid {
            self.cylinder_collider(base, axis.normalize(), radius, length);
        }
    }

    /// Boxes that fit a cylinder closely. Upright: five overlapping
    /// rectangles whose corners lie on the circle (within ~10% of the true
    /// outline, instead of a square around it). Lying down or tilted: a row of short segments, so
    /// a diagonal log doesn't fill a whole square.
    fn cylinder_collider(&mut self, base: Vec3, axis: Vec3, radius: f32, length: f32) {
        if axis.y.abs() > 0.99 {
            let (lo, hi) = (base.y.min(base.y + axis.y * length), base.y.max(base.y + axis.y * length));
            let corners: &[f32] = if radius < 0.35 { &[45.0] } else { &[15.0, 30.0, 45.0, 60.0, 75.0] };
            for &a in corners {
                let (s, c) = a.to_radians().sin_cos();
                let half = Vec3::new(radius * c, 0.0, radius * s);
                self.colliders.push(Aabb {
                    min: Vec3::new(base.x - half.x, lo, base.z - half.z),
                    max: Vec3::new(base.x + half.x, hi, base.z + half.z),
                });
            }
            return;
        }
        let pieces = (length / (radius * 1.5)).ceil().max(1.0) as usize;
        // Cross-section extent of the cylinder along each world axis.
        let reach = Vec3::new(
            radius * (1.0 - axis.x * axis.x).sqrt(),
            radius * (1.0 - axis.y * axis.y).sqrt(),
            radius * (1.0 - axis.z * axis.z).sqrt(),
        );
        for i in 0..pieces {
            let a = base + axis * (length * i as f32 / pieces as f32);
            let b = base + axis * (length * (i + 1) as f32 / pieces as f32);
            self.colliders.push(Aabb { min: a.min(b) - reach, max: a.max(b) + reach });
        }
    }

    /// A round column standing on the terrain at `xz`, from just below the
    /// lowest ground under it up to `bottom + height`.
    #[allow(clippy::too_many_arguments)]
    pub fn round_column(&mut self, surface: Surface, xz: Vec2, radius: f32, bottom: f32, height: f32, tint: Vec4) {
        let base = bottom.min(self.ground_under(xz, Vec2::splat(radius))) - SINK;
        let top = bottom + height;
        self.cylinder(surface, Vec3::new(xz.x, base, xz.y), Vec3::Y, radius, top - base, tint, true);
    }

    /// A solid box from `bottom` up to `bottom + height`, extended down into
    /// the ground so it never floats.
    #[allow(clippy::too_many_arguments)]
    pub fn column(&mut self, surface: Surface, xz: Vec2, footprint: Vec2, bottom: f32, height: f32, tint: Vec4, uv: BoxUv) {
        let base = bottom.min(self.ground_under(xz, footprint * 0.5)) - SINK;
        let top = bottom + height;
        let size = Vec3::new(footprint.x, top - base, footprint.y);
        self.solid(surface, Vec3::new(xz.x, (base + top) * 0.5, xz.y), size, tint, uv);
    }

    /// A wooden crate (optionally with a smaller one on top) on the ground.
    pub fn crate_stack(&mut self, x: f32, z: f32, size: f32, stacked: bool, tint: Vec4) {
        let bottom = self.ground_under(Vec2::new(x, z), Vec2::splat(size * 0.5)) + 0.02;
        let center = Vec3::new(x, bottom + size * 0.5 - 0.06, z);
        self.solid(Surface::Crate, center, Vec3::splat(size), tint, BoxUv::PerFace);
        if stacked {
            let top = size * 0.62;
            let c = Vec3::new(x + size * 0.08, bottom + size + top * 0.5 - 0.06, z - size * 0.06);
            self.solid(Surface::Crate, c, Vec3::splat(top), tint, BoxUv::PerFace);
        }
    }

    /// A plank bench on two stone legs, its seat running along X or Z.
    pub fn bench(&mut self, x: f32, z: f32, along_x: bool) {
        let ground = self.ground(x, z);
        let (len, depth) = (1.8, 0.5);
        let size = if along_x { Vec2::new(len, depth) } else { Vec2::new(depth, len) };
        let leg_offset = if along_x { Vec2::new(0.65, 0.0) } else { Vec2::new(0.0, 0.65) };
        for s in [-1.0, 1.0] {
            let leg = Vec2::new(x, z) + leg_offset * s;
            let leg_size = if along_x { Vec2::new(0.25, 0.4) } else { Vec2::new(0.4, 0.25) };
            self.column(Surface::StoneWall, leg, leg_size, ground, 0.42, Vec4::ONE, BoxUv::World(1.0));
        }
        self.solid(
            Surface::Planks,
            Vec3::new(x, ground + 0.47, z),
            Vec3::new(size.x, 0.1, size.y),
            Vec4::new(0.85, 0.75, 0.65, 1.0),
            BoxUv::World(2.0),
        );
    }

    /// A wooden fence along an axis-aligned polyline, with posts every ~2 m.
    pub fn fence(&mut self, points: &[Vec2]) {
        let wood = Vec4::new(0.75, 0.62, 0.5, 1.0);
        for w in points.windows(2) {
            let (a, b) = (w[0], w[1]);
            let n = ((b - a).length() / 2.0).ceil().max(1.0) as usize;
            for i in 0..=n {
                let p = a + (b - a) * (i as f32 / n as f32);
                let g = self.ground(p.x, p.y);
                self.column(Surface::Planks, p, Vec2::splat(0.14), g, 1.1, wood, BoxUv::World(1.0));
                if i < n {
                    let q = a + (b - a) * ((i + 1) as f32 / n as f32);
                    let mid = (p + q) * 0.5;
                    let g_mid = (g + self.ground(q.x, q.y)) * 0.5;
                    let length = (q - p).length();
                    let size = if (q - p).x.abs() > (q - p).y.abs() {
                        Vec3::new(length, 0.08, 0.06)
                    } else {
                        Vec3::new(0.06, 0.08, length)
                    };
                    for h in [0.5, 0.9] {
                        self.visual(Surface::Planks, Vec3::new(mid.x, g_mid + h, mid.y), size, wood, BoxUv::World(1.0));
                    }
                    // One collider per span so the fence can't be walked through.
                    let blocker = Vec3::new(size.x.max(0.2), 1.0, size.z.max(0.2));
                    self.colliders
                        .push(Aabb::from_center(Vec3::new(mid.x, g_mid + 0.5, mid.y), blocker));
                }
            }
        }
    }

    /// A pitched roof: shingled slopes and gable triangles (in
    /// `gable_surface`) at both ends. `half` is (half length along the
    /// ridge, half width across it), measured to the outside of the walls.
    #[allow(clippy::too_many_arguments)]
    pub fn gable_roof(
        &mut self,
        center: Vec2,
        half: Vec2,
        ridge_along_x: bool,
        base_y: f32,
        rise: f32,
        overhang: f32,
        tint: Vec4,
        gable_surface: Surface,
        gable_tint: Vec4,
    ) {
        // Local (u along the ridge, v across it) to world.
        let world = |u: f32, v: f32, y: f32| {
            if ridge_along_x {
                Vec3::new(center.x + u, y, center.y + v)
            } else {
                Vec3::new(center.x + v, y, center.y + u)
            }
        };
        let dir = |u: f32, y: f32, v: f32| if ridge_along_x { Vec3::new(u, y, v) } else { Vec3::new(v, y, u) };
        let (a, b) = (half.x + overhang, half.y + overhang);
        let ridge_y = base_y + rise;
        // Solid roof: a stack of slabs that narrows towards the ridge, so you
        // stand on the slopes instead of sinking into them (each step is
        // low enough to walk up).
        let layers = (rise / 0.3).ceil().max(1.0) as usize;
        for i in 0..layers {
            let (y0, y1) = (base_y + rise * i as f32 / layers as f32, base_y + rise * (i + 1) as f32 / layers as f32);
            let across = half.y * (1.0 - (i as f32 + 0.5) / layers as f32);
            let (lo, hi) = (world(-a, -across, y0), world(a, across, y1));
            self.colliders.push(Aabb { min: lo.min(hi), max: lo.max(hi) });
        }
        let eave_y = base_y - rise * overhang / half.y;
        let slope_len = b.hypot(ridge_y - eave_y);
        for side in [-1.0f32, 1.0] {
            let corners = [
                world(-a, 0.0, ridge_y),
                world(a, 0.0, ridge_y),
                world(a, side * b, eave_y),
                world(-a, side * b, eave_y),
            ];
            let uv = [
                Vec2::new(-a, 0.0),
                Vec2::new(a, 0.0),
                Vec2::new(a, slope_len),
                Vec2::new(-a, slope_len),
            ]
            .map(|v| v / 2.0);
            self.mesh(Surface::Shingles)
                .push_quad(corners, uv, tint, dir(0.0, 1.0, side));
        }
        for end in [-1.0f32, 1.0] {
            let u = end * half.x;
            let p = [
                world(u, -half.y, base_y),
                world(u, half.y, base_y),
                world(u, 0.0, ridge_y),
            ];
            let uv = [Vec2::new(-half.y, 0.0), Vec2::new(half.y, 0.0), Vec2::new(0.0, -rise)].map(|v| v / 2.0);
            self.mesh(gable_surface)
                .push_tri(p, uv, gable_tint, dir(end, 0.0, 0.0));
        }
    }

    /// Builds a house on the terrain (on its lowest point, with walls
    /// reaching down into the ground on slopes).
    pub fn house(&mut self, h: &House) {
        let half = h.size * 0.5;
        let floor = self.ground_under(h.center, half) + 0.05;
        let top = floor + h.wall_height;
        let t = 0.4; // wall thickness
        let door_width = 1.6;
        let door_height = 2.3;

        for side in [Side::North, Side::South, Side::East, Side::West] {
            let n = side.normal();
            let along_x = n.y != 0.0;
            let length = if along_x { h.size.x } else { h.size.y - 2.0 * t };
            let mid = h.center + n * (if along_x { half.y } else { half.x } - t * 0.5);
            let dims = |len: f32| if along_x { Vec2::new(len, t) } else { Vec2::new(t, len) };
            if side == h.door {
                let piece = (length - door_width) * 0.5;
                let offset = (door_width + piece) * 0.5;
                let axis = if along_x { Vec2::X } else { Vec2::Y };
                for s in [-1.0, 1.0] {
                    let c = mid + axis * offset * s;
                    self.column(h.wall, c, dims(piece), floor, h.wall_height, h.wall_tint, BoxUv::World(2.0));
                }
                // Lintel over the door.
                let lintel = top - (floor + door_height);
                let size = dims(door_width);
                self.solid(
                    h.wall,
                    Vec3::new(mid.x, floor + door_height + lintel * 0.5, mid.y),
                    Vec3::new(size.x, lintel, size.y),
                    h.wall_tint,
                    BoxUv::World(2.0),
                );
            } else {
                self.column(h.wall, mid, dims(length), floor, h.wall_height, h.wall_tint, BoxUv::World(2.0));
            }
        }

        // Wooden floor inside.
        let inner = h.size - Vec2::splat(2.0 * t);
        self.visual(
            Surface::Planks,
            Vec3::new(h.center.x, floor + 0.02, h.center.y),
            Vec3::new(inner.x, 0.04, inner.y),
            Vec4::ONE,
            BoxUv::World(2.0),
        );

        // Roof, ridge parallel to the door wall.
        let ridge_along_x = matches!(h.door, Side::North | Side::South);
        let roof_half = if ridge_along_x { half } else { Vec2::new(half.y, half.x) };
        self.gable_roof(h.center, roof_half, ridge_along_x, top, h.roof_rise, 0.5, h.roof_tint, h.wall, h.wall_tint);
        // Wall plates: fill the wedge between the thick eave walls and the
        // sloping roof so no daylight shows through.
        for s in [-1.0, 1.0] {
            let offset = roof_half.y - 0.5;
            let (c, size) = if ridge_along_x {
                (Vec2::new(h.center.x, h.center.y + s * offset), Vec3::new(inner.x, 0.2, 0.2))
            } else {
                (Vec2::new(h.center.x + s * offset, h.center.y), Vec3::new(0.2, 0.2, inner.y))
            };
            self.visual(h.wall, Vec3::new(c.x, top + 0.1, c.y), size, h.wall_tint, BoxUv::World(2.0));
        }
    }

    /// Places a model file (path relative to `assets/`). Each file is loaded
    /// once however many times it is placed. Returns `None` if the file
    /// failed to load (the error is logged and the scene carries on without it).
    pub fn place(&mut self, file: &str, p: Placement) -> Option<()> {
        let index = *self.loaded.entry(file.to_owned()).or_insert_with(|| {
            match model::load(&self.assets.join(file)) {
                Ok(model) => {
                    log::info!("loaded {file}: {} part(s)", model.parts.len());
                    self.models.push(PlacedModel {
                        model,
                        instances: Vec::new(),
                    });
                    Some(self.models.len() - 1)
                }
                Err(e) => {
                    log::error!("{e:#}");
                    None
                }
            }
        });
        let index = index?;
        let y = p
            .y
            .unwrap_or_else(|| self.ground(p.position.x, p.position.y) - 0.08);
        let position = Vec3::new(p.position.x, y, p.position.y);
        let placed = &mut self.models[index];

        let b = placed.model.bounds;
        let size = b.size();
        let bottom_center = Vec3::new((b.min.x + b.max.x) * 0.5, b.min.y, (b.min.z + b.max.z) * 0.5);
        let fit = match p.height {
            Some(h) if size.y > 0.0 => h / size.y,
            _ => 1.0,
        };
        let transform = Mat4::from_scale_rotation_translation(
            Vec3::splat(fit * p.scale),
            Quat::from_rotation_y(-p.yaw_degrees.to_radians()),
            position,
        ) * Mat4::from_translation(-bottom_center);
        placed.instances.push(Instance { transform, tint: p.tint });

        match p.collider {
            Collider::None => {}
            Collider::Bounds => {
                let boxes = fitted_boxes(&placed.model, transform);
                self.colliders.extend(boxes);
            }
            Collider::Box(size) => {
                let origin = transform.transform_point3(Vec3::ZERO);
                let center = Vec3::new(origin.x, position.y + size.y * 0.5, origin.z);
                self.colliders.push(Aabb::from_center(center, size));
            }
        }
        Some(())
    }

    /// The finished terrain, static geometry (one part per surface), models
    /// and colliders.
    pub fn finish(
        self,
        materials: &HashMap<Surface, MaterialData>,
    ) -> (Terrain, Vec<ModelPart>, Vec<PlacedModel>, Vec<Aabb>) {
        let parts = self
            .surfaces
            .into_iter()
            .map(|(surface, mesh)| ModelPart {
                mesh,
                material: materials[&surface].clone(),
            })
            .collect();
        (self.terrain, parts, self.models, self.colliders)
    }
}

pub fn build() -> Scene {
    crate::map::build()
}

/// Colliders that follow a placed model's real shape. The model is cut into
/// slices along its longest horizontal side, and each slice gets the box
/// around its own vertices in world space. A rotated rock or a fallen log
/// then gets a snug fit instead of one big box around the turned model.
fn fitted_boxes(model: &Model, transform: Mat4) -> Vec<Aabb> {
    let b = model.bounds;
    let size = b.size();
    let along_x = size.x >= size.z;
    let (long, short) = if along_x { (size.x, size.z) } else { (size.z, size.x) };
    let start = if along_x { b.min.x } else { b.min.z };
    let slices = (long / short.max(0.5)).ceil().clamp(1.0, 12.0) as usize;
    let slice_len = long / slices as f32;
    let mut boxes: Vec<Option<Aabb>> = vec![None; slices];
    // Big scanned models: a sample of the vertices is plenty.
    let count: usize = model.parts.iter().map(|p| p.mesh.vertices.len()).sum();
    let stride = (count / 4000).max(1);
    for v in model.parts.iter().flat_map(|p| p.mesh.vertices.iter()).step_by(stride) {
        let local = Vec3::from(v.position);
        let t = if along_x { local.x } else { local.z };
        let i = (((t - start) / slice_len) as usize).min(slices - 1);
        let w = transform.transform_point3(local);
        let bx = boxes[i].get_or_insert(Aabb { min: w, max: w });
        bx.min = bx.min.min(w);
        bx.max = bx.max.max(w);
    }
    boxes
        .into_iter()
        .enumerate()
        .map(|(i, bx)| {
            // A slice with no vertices (a long face spans it): use the
            // slice of the bounding box.
            bx.unwrap_or_else(|| {
                let (mut lo, mut hi) = (b.min, b.max);
                let (s0, s1) = (start + slice_len * i as f32, start + slice_len * (i + 1) as f32);
                if along_x {
                    (lo.x, hi.x) = (s0, s1);
                } else {
                    (lo.z, hi.z) = (s0, s1);
                }
                Aabb { min: lo, max: hi }.transformed(transform)
            })
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;

    fn solid_at(colliders: &[Aabb], p: Vec3) -> bool {
        let probe = Aabb::from_center(p, Vec3::splat(0.02));
        colliders.iter().any(|c| c.intersects(&probe))
    }

    #[test]
    fn round_columns_are_round() {
        let mut w = SceneBuilder::new(Terrain::flat(50.0));
        w.cylinder(Surface::StoneWall, Vec3::ZERO, Vec3::Y, 4.0, 4.0, Vec4::ONE, true);
        assert!(solid_at(&w.colliders, Vec3::new(3.8, 1.0, 0.0)), "inside the edge");
        assert!(solid_at(&w.colliders, Vec3::new(2.6, 1.0, 2.6)), "inside, diagonally");
        // A square box would block here, outside the circle at the corner.
        assert!(!solid_at(&w.colliders, Vec3::new(3.4, 1.0, 3.4)));
    }

    #[test]
    fn diagonal_logs_dont_fill_a_square() {
        let mut w = SceneBuilder::new(Terrain::flat(50.0));
        let axis = Vec3::new(1.0, 0.0, 1.0).normalize();
        w.cylinder(Surface::Planks, Vec3::new(0.0, 0.5, 0.0), axis, 0.5, 6.0, Vec4::ONE, true);
        assert!(solid_at(&w.colliders, Vec3::new(2.0, 0.5, 2.0)), "on the log");
        assert!(!solid_at(&w.colliders, Vec3::new(3.5, 0.5, 0.5)), "beside the log");
    }

    #[test]
    fn roofs_can_be_stood_on() {
        let mut w = SceneBuilder::new(Terrain::flat(50.0));
        let (base, rise) = (3.0, 2.0);
        w.gable_roof(Vec2::ZERO, Vec2::new(4.0, 3.0), true, base, rise, 0.5, Vec4::ONE, Surface::Brick, Vec4::ONE);
        // Highest solid point above a spot on the slope, halfway up.
        let top_at = |z: f32| {
            w.colliders
                .iter()
                .filter(|c| c.min.z <= z && c.max.z >= z && c.min.x <= 0.0 && c.max.x >= 0.0)
                .map(|c| c.max.y)
                .fold(f32::MIN, f32::max)
        };
        let roof_surface = base + rise * 0.5;
        assert!((top_at(1.5) - roof_surface).abs() < 0.35, "{}", top_at(1.5));
        assert!((top_at(0.0) - (base + rise)).abs() < 0.01, "ridge");
    }
}
