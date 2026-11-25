//! The world map, designed by hand.
//!
//! A 400 m valley ringed by mountains. A lake fills the west, fed by a river
//! that winds down out of the north-east mountains. Paths link the places
//! you can visit into loops:
//!
//! - **Village** (spawn): a cobbled square with a well, four houses facing
//!   it, a market corner, a fenced flower garden. The view from the square
//!   looks out over the lake.
//! - **Pier** reaching into the lake from the village shore.
//! - **Bridge** over the river, into the **northern pine forest**.
//! - **Watchtower** on a bluff above the north-west shore, reached by a path
//!   that winds up around the hill.
//! - **Lakeshore walk** round the south and west shore, past the
//!   **fisher's rest**, back up to the tower.
//! - **Stone circle** ruins on the eastern hilltop.
//! - **Wildflower meadow** south of the ruins, ending at a **viewpoint**.
//!
//! Terrain heights are built in layers: rolling land, named hills, level
//! pads under every structure, the mountain ring, then the lake and river
//! carved in, so everything sits naturally in the land.

use std::collections::HashMap;
use std::f32::consts::TAU;
use std::time::Instant;

use glam::{Vec2, Vec3, Vec4};

use crate::collision::Aabb;
use crate::mesh::BoxUv;
use crate::model::{MaterialData, TextureData};
use crate::noise::{fbm, hash2, lerp, ridged, smoothstep};
use crate::renderer::{GrassField, Lighting};
use crate::scene::{Collider, House, Placement, Scene, SceneBuilder, Side, surface_material};
use crate::terrain::{DistanceField, Terrain, spline};
use crate::textures::{self, Surface, SurfaceTextures};

pub const HALF: f32 = 200.0;
pub const WATER: f32 = 0.0;

// --- places -----------------------------------------------------------------

const VILLAGE: Vec2 = Vec2::new(0.0, 40.0);
const LAKE_CENTER: Vec2 = Vec2::new(-62.0, -22.0);
const LAKE_RADII: Vec2 = Vec2::new(56.0, 40.0);
/// Rotation of the lake's ellipse, radians.
const LAKE_ANGLE: f32 = 0.35;
const PIER_Z: f32 = 8.0;
const BRIDGE_X: f32 = 25.0;
/// Bridge deck runs between these Z coordinates (both banks).
const BRIDGE_ENDS: (f32, f32) = (-71.0, -39.0);
const TOWER: Vec2 = Vec2::new(-110.0, -108.0);
const CIRCLE: Vec2 = Vec2::new(95.0, 55.0);
const MEADOW: Vec2 = Vec2::new(78.0, 100.0);
const VIEWPOINT: Vec2 = Vec2::new(92.0, 120.0);
const FISHER: Vec2 = Vec2::new(-114.0, -4.0);
/// Fenced garden behind the village (min, max corners).
const GARDEN: (Vec2, Vec2) = (Vec2::new(-26.0, 48.0), Vec2::new(-16.0, 58.0));

struct Hill {
    center: Vec2,
    radius: f32,
    height: f32,
}

const HILLS: [Hill; 6] = [
    // Stone circle hill.
    Hill { center: CIRCLE, radius: 52.0, height: 15.0 },
    // The tower's bluff.
    Hill { center: TOWER, radius: 40.0, height: 17.0 },
    // Forested ridge along the north.
    Hill { center: Vec2::new(-20.0, -150.0), radius: 70.0, height: 12.0 },
    // Wooded hill east of the river.
    Hill { center: Vec2::new(138.0, -35.0), radius: 50.0, height: 10.0 },
    // Rolling high ground south-west of the lake.
    Hill { center: Vec2::new(-120.0, 90.0), radius: 55.0, height: 11.0 },
    // A knoll beside the meadow.
    Hill { center: Vec2::new(45.0, 112.0), radius: 25.0, height: 5.0 },
];

/// Flattened ground under a structure.
struct Pad {
    center: Vec2,
    /// Half size for a rectangle; for a circle `half.x` is the radius and
    /// `half.y` is negative.
    half: Vec2,
    blend: f32,
    height: f32,
}

impl Pad {
    fn rect(center: Vec2, half: Vec2, blend: f32, height: f32) -> Self {
        Self { center, half, blend, height }
    }
    fn round(center: Vec2, radius: f32, blend: f32, height: f32) -> Self {
        Self {
            center,
            half: Vec2::new(radius, -1.0),
            blend,
            height,
        }
    }
    /// 1 inside, fading to 0 over `blend` meters outside.
    fn weight(&self, p: Vec2) -> f32 {
        let d = p - self.center;
        let outside = if self.half.y < 0.0 {
            (d.length() - self.half.x).max(0.0)
        } else {
            let q = d.abs() - self.half;
            q.max(Vec2::ZERO).length()
        };
        1.0 - smoothstep(0.0, self.blend, outside)
    }
}

fn path_points() -> Vec<(Vec<Vec2>, f32)> {
    let v = |pts: &[(f32, f32)]| pts.iter().map(|&(x, z)| Vec2::new(x, z)).collect::<Vec<_>>();
    vec![
        // Village down to the pier.
        (v(&[(-6.0, 34.0), (-10.0, 24.0), (-13.0, 15.0), (-14.0, 9.0)]), 1.3),
        // Village north to the bridge.
        (v(&[(2.0, 30.0), (4.0, 16.0), (9.0, 0.0), (16.0, -18.0), (22.0, -32.0), (25.0, -39.0)]), 1.5),
        // Over the river, through the forest, winding up the tower bluff.
        (
            v(&[
                (25.0, -71.0),
                (23.0, -86.0),
                (12.0, -102.0),
                (-10.0, -116.0),
                (-40.0, -126.0),
                (-66.0, -126.0),
                (-82.0, -124.0),
                (-96.0, -128.0),
                (-114.0, -126.0),
                (-124.0, -114.0),
                (-123.0, -100.0),
                (-116.0, -92.0),
                (-110.0, -95.0),
            ]),
            1.4,
        ),
        // Village east, climbing to the stone circle.
        (
            v(&[
                (12.0, 48.0),
                (28.0, 51.0),
                (46.0, 52.0),
                (62.0, 60.0),
                (76.0, 70.0),
                (90.0, 72.0),
                (102.0, 66.0),
                (104.0, 57.0),
                (101.0, 52.0),
            ]),
            1.4,
        ),
        // Stone circle down through the meadow to the viewpoint.
        (v(&[(100.0, 66.0), (106.0, 82.0), (101.0, 98.0), (95.0, 111.0), (92.0, 117.0)]), 1.2),
        // Lakeshore walk: pier, south shore, fisher's rest, up to the tower path.
        (
            v(&[
                (-14.0, 9.0),
                (-30.0, 22.0),
                (-52.0, 30.0),
                (-76.0, 27.0),
                (-98.0, 16.0),
                (-114.0, 0.0),
                (-126.0, -24.0),
                (-127.0, -52.0),
                (-122.0, -76.0),
                (-124.0, -100.0),
            ]),
            1.2,
        ),
    ]
}

fn river_points() -> Vec<Vec2> {
    [
        (138.0, -170.0),
        (120.0, -130.0),
        (92.0, -104.0),
        (60.0, -84.0),
        (25.0, -55.0),
        (2.0, -46.0),
        (-16.0, -40.0),
        (-40.0, -32.0),
    ]
    .iter()
    .map(|&(x, z)| Vec2::new(x, z))
    .collect()
}

/// The authored layout plus precomputed distance fields, used to shape the
/// terrain and to decide what grows and what the ground looks like.
pub struct Layout {
    /// Distance to the river's centre line.
    river: DistanceField,
    /// Distance to the nearest path edge (negative on a path).
    path: DistanceField,
    /// Distance to paved areas (negative on paving).
    paving: DistanceField,
    pads: Vec<Pad>,
    /// Pads applied after carving the river (the bridge banks).
    bank_pads: Vec<Pad>,
}

impl Layout {
    pub fn new() -> Self {
        let mut river = DistanceField::new(HALF, 1.0);
        river.add(&spline(&river_points(), 1.0), 0.0, 32.0);

        let mut path = DistanceField::new(HALF, 1.0);
        for (points, half_width) in path_points() {
            path.add(&spline(&points, 1.0), half_width, 6.0);
        }

        let mut paving = DistanceField::new(HALF, 0.5);
        // Village square, stone circle floor, tower top.
        paving.add(&[VILLAGE - Vec2::new(6.0, 0.0), VILLAGE + Vec2::new(6.0, 0.0)], 7.0, 3.0);
        paving.add(&[CIRCLE], 11.0, 3.0);
        paving.add(&[TOWER + Vec2::new(0.0, -3.0)], 5.0, 3.0);

        let mut layout = Self {
            river,
            path,
            paving,
            pads: Vec::new(),
            bank_pads: Vec::new(),
        };
        let natural = |p: Vec2| layout.natural(p);
        let pads = vec![
            Pad::rect(VILLAGE + Vec2::new(2.0, 2.0), Vec2::new(25.0, 22.0), 14.0, natural(VILLAGE).max(3.0)),
            Pad::round(CIRCLE, 12.0, 8.0, natural(CIRCLE)),
            Pad::round(TOWER + Vec2::new(0.0, 2.0), 12.0, 6.0, natural(TOWER)),
            Pad::round(VIEWPOINT, 5.0, 6.0, natural(VIEWPOINT)),
            Pad::round(FISHER, 5.0, 6.0, natural(FISHER)),
        ];
        layout.pads = pads;
        // Both bridge banks levelled to the same height so the deck is flat.
        let bank = |z: f32| layout.carved(Vec2::new(BRIDGE_X, z));
        let bank_height = bank(BRIDGE_ENDS.0).max(bank(BRIDGE_ENDS.1)).max(WATER + 1.5);
        layout.bank_pads = vec![
            Pad::round(Vec2::new(BRIDGE_X, BRIDGE_ENDS.0 - 2.0), 3.5, 7.0, bank_height),
            Pad::round(Vec2::new(BRIDGE_X, BRIDGE_ENDS.1 + 2.0), 3.5, 7.0, bank_height),
        ];
        layout
    }

    /// Rolling land and hills, with fine bumps smoothed away on paths.
    fn natural(&self, p: Vec2) -> f32 {
        let path_w = 1.0 - smoothstep(0.0, 2.5, self.path.at(p));
        let mut h = 1.5 + fbm(p * 0.0065, 5, 1) * 12.0;
        h += (fbm(p * 0.07, 3, 2) - 0.5) * 1.4 * (1.0 - path_w);
        for hill in &HILLS {
            let t = (p - hill.center).length() / hill.radius;
            h += hill.height * (1.0 - smoothstep(0.0, 1.0, t));
        }
        // The land slopes gently down towards the lake.
        h - (1.0 - smoothstep(40.0, 160.0, (p - LAKE_CENTER).length())) * 4.0
    }

    /// 0 at the lake's centre, 1 at its shore (with a wavy outline).
    fn lake_distance(&self, p: Vec2) -> f32 {
        let q = p - LAKE_CENTER;
        let (s, c) = (-LAKE_ANGLE).sin_cos();
        let r = Vec2::new(q.x * c - q.y * s, q.x * s + q.y * c) / LAKE_RADII;
        r.length() + (fbm(p * 0.025, 3, 7) - 0.5) * 0.22
    }

    /// Rounded-square distance from the map centre (for the mountain ring).
    fn edge_distance(p: Vec2) -> f32 {
        (p.x.powi(4) + p.y.powi(4)).powf(0.25)
    }

    /// Everything up to and including the lake and river.
    fn carved(&self, p: Vec2) -> f32 {
        let mut h = self.natural(p);
        for pad in &self.pads {
            h = lerp(h, pad.height, pad.weight(p));
        }

        // Mountains rise steeply all round the edge.
        // Foothills first, then the peaks climbing behind them.
        let m = smoothstep(135.0, 198.0, Self::edge_distance(p));
        h += m * m * (40.0 + ridged(p * 0.012, 5, 3) * 45.0) + m * ridged(p * 0.02, 4, 4) * 10.0;

        // The lake: a sandy bowl, with the land easing down to its shore.
        let d = self.lake_distance(p);
        h = if d < 1.0 {
            lerp(-4.5, 0.45, smoothstep(0.45, 1.0, d))
        } else {
            lerp(0.45, h, smoothstep(1.0, 1.55, d))
        };

        // The river: a channel below the water line in a gently sloped valley.
        let dr = self.river.at(p);
        if dr < 30.0 {
            let channel = lerp(-1.4, 0.6, smoothstep(1.5, 5.0, dr));
            h = h.min(lerp(channel, h, smoothstep(5.0, 26.0, dr)));
        }
        h
    }

    pub fn height(&self, p: Vec2) -> f32 {
        let mut h = self.carved(p);
        for pad in &self.bank_pads {
            h = lerp(h, pad.height, pad.weight(p));
        }
        h
    }

    /// 0..1: how forested a spot is (before checking paths and buildings).
    fn forest_density(&self, p: Vec2) -> f32 {
        let n = fbm(p * 0.02, 4, 21);
        let north = smoothstep(-70.0, -100.0, p.y) * smoothstep(0.3, 0.45, n);
        let east = (1.0 - smoothstep(30.0, 70.0, (p - Vec2::new(138.0, -35.0)).length())) * smoothstep(0.3, 0.42, n);
        let south_west = (1.0 - smoothstep(25.0, 60.0, (p - Vec2::new(-120.0, 90.0)).length())) * smoothstep(0.35, 0.5, n);
        let groves = smoothstep(0.6, 0.7, n) * 0.8;
        let base = north.max(east).max(south_west).max(groves);

        // Keep the special places open, with room to see.
        let open = [
            1.0 - smoothstep(35.0, 50.0, (p - VILLAGE).length()),
            1.0 - smoothstep(30.0, 55.0, (p - MEADOW).length()),
            1.0 - smoothstep(16.0, 26.0, (p - CIRCLE).length()),
            // Wide, so the tall pines don't block the view from the tower.
            1.0 - smoothstep(26.0, 36.0, (p - TOWER).length()),
            1.0 - smoothstep(1.05, 1.25, self.lake_distance(p)),
            1.0 - smoothstep(6.0, 12.0, self.river.at(p)),
            smoothstep(176.0, 186.0, Self::edge_distance(p)),
        ]
        .into_iter()
        .fold(0.0f32, f32::max);
        base * (1.0 - open)
    }

    /// 0..1: chance a grass blade is a wildflower.
    fn flower_density(&self, p: Vec2) -> f32 {
        let meadow = 1.0 - smoothstep(25.0, 55.0, (p - MEADOW).length());
        let drifts = smoothstep(0.4, 0.62, fbm(p * 0.05, 3, 31));
        let sprinkles = smoothstep(0.58, 0.72, fbm(p * 0.03, 3, 32)) * 0.04;
        let garden = garden_weight(p) * 0.4;
        (meadow * drifts * 0.18 + sprinkles).max(garden)
    }

    /// Ground layer weights at a point: dirt, rock, sand, paving (grass is
    /// whatever is left).
    fn ground_layers(&self, p: Vec2, terrain: &Terrain) -> [f32; 4] {
        let h = terrain.height_at(p.x, p.y);
        let slope = 1.0 - terrain.normal_at(p.x, p.y).y;
        let noise = fbm(p * 0.35, 3, 11) - 0.5;

        let paved = 1.0 - smoothstep(-0.3, 0.3, self.paving.at(p) + noise * 0.8);
        let path = 1.0 - smoothstep(-0.5, 0.5, self.path.at(p) + noise * 1.0);
        let litter = self.forest_density(p) * smoothstep(0.42, 0.72, fbm(p * 0.12, 3, 12)) * 0.8;
        let dirt = path.max(litter).max(garden_weight(p));
        let rock = smoothstep(0.2, 0.34, slope + noise * 0.08).max(smoothstep(44.0, 60.0, h + noise * 8.0));
        let sand = 1.0 - smoothstep(0.2, 1.2, h - WATER + noise * 0.6);

        // Priority: paving, then dirt, then rock, then sand.
        let a = paved;
        let r = dirt * (1.0 - a);
        let g = rock * (1.0 - a - r).max(0.0);
        let b = sand * (1.0 - a - r - g).max(0.0);
        [r, g, b, a]
    }

    fn splat(&self, terrain: &Terrain) -> TextureData {
        const RES: usize = 2048;
        let rgba = world_image(RES, |p| self.ground_layers(p, terrain).map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8));
        TextureData {
            width: RES as u32,
            height: RES as u32,
            rgba,
            srgb: false,
        }
    }

    /// Red: grass density. Green: wildflower chance.
    fn grass_mask(&self, terrain: &Terrain, colliders: &[Aabb]) -> TextureData {
        const RES: usize = 1024;
        let mut rgba = world_image(RES, |p| {
            let covered: f32 = self.ground_layers(p, terrain).iter().sum();
            let h = terrain.height_at(p.x, p.y);
            let dry_land = smoothstep(WATER + 0.1, WATER + 0.4, h);
            let shade = 1.0 - 0.55 * self.forest_density(p);
            // Flowers also grow in the garden's dirt.
            let garden = garden_weight(p);
            let grass = ((1.0 - covered * 1.4).max(garden * 0.6) * dry_land * shade).clamp(0.0, 1.0);
            let flowers = self.flower_density(p);
            [(grass * 255.0) as u8, (flowers * 255.0) as u8, 0, 255]
        });
        // Nothing grows inside or just under anything (houses, crates, the
        // ends of the bridge), judged against the ground at each spot.
        let cell = 2.0 * HALF / RES as f32;
        for c in colliders {
            let to_cell = |v: f32| ((v + HALF) / cell).floor() as i64;
            for z in to_cell(c.min.z - 0.05).max(0)..=to_cell(c.max.z + 0.05).min(RES as i64 - 1) {
                for x in to_cell(c.min.x - 0.05).max(0)..=to_cell(c.max.x + 0.05).min(RES as i64 - 1) {
                    let wx = -HALF + (x as f32 + 0.5) * cell;
                    let wz = -HALF + (z as f32 + 0.5) * cell;
                    let ground = terrain.height_at(wx, wz);
                    if c.min.y < ground + 0.6 && c.max.y > ground {
                        rgba[(z as usize * RES + x as usize) * 4] = 0;
                    }
                }
            }
        }
        TextureData {
            width: RES as u32,
            height: RES as u32,
            rgba,
            srgb: false,
        }
    }

    /// True if `p` is clear of paths, paving and water, with room `margin`.
    fn open_ground(&self, p: Vec2, margin: f32) -> bool {
        self.path.at(p) > margin && self.paving.at(p) > margin + 1.0 && self.river.at(p) > 6.0 + margin
    }
}

impl Default for Layout {
    fn default() -> Self {
        Self::new()
    }
}

fn garden_weight(p: Vec2) -> f32 {
    let (lo, hi) = GARDEN;
    let inside = p.x > lo.x + 0.5 && p.x < hi.x - 0.5 && p.y > lo.y + 0.5 && p.y < hi.y - 0.5;
    if inside { 1.0 } else { 0.0 }
}

/// An RGBA image covering the whole world, one texel per `2 * HALF / res`
/// meters, filled in parallel.
fn world_image(res: usize, texel: impl Fn(Vec2) -> [u8; 4] + Sync) -> Vec<u8> {
    let mut data = vec![0u8; res * res * 4];
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let rows_per = res.div_ceil(threads);
    std::thread::scope(|s| {
        for (chunk_index, chunk) in data.chunks_mut(rows_per * res * 4).enumerate() {
            let texel = &texel;
            s.spawn(move || {
                for (i, px) in chunk.chunks_exact_mut(4).enumerate() {
                    let index = chunk_index * rows_per * res + i;
                    let (x, z) = (index % res, index / res);
                    let p = Vec2::new(
                        -HALF + (x as f32 + 0.5) / res as f32 * 2.0 * HALF,
                        -HALF + (z as f32 + 0.5) / res as f32 * 2.0 * HALF,
                    );
                    px.copy_from_slice(&texel(p));
                }
            });
        }
    });
    data
}

// --- structures -------------------------------------------------------------

const LANTERN: &str = "models/lantern/Lantern.glb";
// Game-ready low-poly trees from flo-bit's low poly nature pack (CC0,
// https://github.com/flo-bit/low-poly-asset-packs), fetched by
// tools/fetch_assets.ps1 into assets/trees. Three variants of each kind.
const COMMON: [&str; 3] = ["trees/common_tree_1.glb", "trees/common_tree_2.glb", "trees/common_tree_3.glb"];
const COMMON_AUTUMN: [&str; 3] = [
    "trees/common_tree_1_autumn.glb",
    "trees/common_tree_2_autumn.glb",
    "trees/common_tree_3_autumn.glb",
];
const PINES: [&str; 3] = ["trees/pine_tree_1.glb", "trees/pine_tree_2.glb", "trees/pine_tree_3.glb"];
const PINES_AUTUMN: [&str; 3] = [
    "trees/pine_tree_1_autumn.glb",
    "trees/pine_tree_2_autumn.glb",
    "trees/pine_tree_3_autumn.glb",
];
const PINES_SNOW: [&str; 3] = ["trees/pine_tree_1_snow.glb", "trees/pine_tree_2_snow.glb", "trees/pine_tree_3_snow.glb"];
const DEAD: [&str; 6] = [
    "trees/dead_tree_1.glb",
    "trees/dead_tree_2.glb",
    "trees/dead_tree_3.glb",
    "trees/common_tree_dead_1.glb",
    "trees/common_tree_dead_2.glb",
    "trees/common_tree_dead_3.glb",
];
const BUSHES: [&str; 5] = ["trees/bush_1.glb", "trees/bush_2.glb", "trees/bush_3.glb", "trees/bush_4.glb", "trees/bush_5.glb"];

struct Species {
    file: &'static str,
    /// Typical height in meters (each tree is scaled around it).
    height: f32,
    /// Width of the solid trunk collider.
    trunk: f32,
}

/// Which tree grows where: each part of the valley has its own woodland.
/// `lone`: standing alone in open land. `ground`: terrain height there.
/// `roll` picks the kind, `variant` one of its models (both 0..1).
fn species_at(p: Vec2, lone: bool, ground: f32, roll: f32, variant: f32) -> Species {
    let pick = |set: &[&'static str]| set[((variant * set.len() as f32) as usize).min(set.len() - 1)];
    let broadleaf = |set: &[&'static str]| Species { file: pick(set), height: 8.5, trunk: 0.7 };
    let conifer = |set: &[&'static str]| Species { file: pick(set), height: 11.0, trunk: 0.55 };
    let dead = || Species { file: pick(&DEAD), height: 6.5, trunk: 0.5 };

    if lone {
        // Spreading broadleaf trees in the fields and along the water.
        return if roll < 0.85 { broadleaf(&COMMON) } else { conifer(&PINES) };
    }
    if p.y < -80.0 {
        // The northern forest: pines, snow-dusted high on the slopes.
        let pines = if ground > 24.0 { &PINES_SNOW } else { &PINES };
        return match roll {
            r if r < 0.8 => conifer(pines),
            r if r < 0.9 => broadleaf(&COMMON),
            _ => dead(),
        };
    }
    if (p - Vec2::new(138.0, -35.0)).length() < 75.0 {
        // The east hill: an autumn wood.
        return match roll {
            r if r < 0.6 => broadleaf(&COMMON_AUTUMN),
            r if r < 0.88 => conifer(&PINES_AUTUMN),
            _ => dead(),
        };
    }
    // Everywhere else: mixed woodland.
    match roll {
        r if r < 0.55 => broadleaf(&COMMON),
        r if r < 0.92 => conifer(&PINES),
        _ => dead(),
    }
}const ROCK: &str = "models/rock.obj";
// Photo-scanned CC0 models from Poly Haven (tools/fetch_assets.ps1). Missing
// files are skipped (with a logged error), so the world still builds.
const BOULDER: &str = "polyhaven/models/boulder_01/boulder_01.gltf";
const MOSS_ROCKS: &str = "polyhaven/models/rock_moss_set_01/rock_moss_set_01.gltf";
const SMALL_ROCK: &str = "polyhaven/models/rock_07/rock_07.gltf";
const FERN: &str = "polyhaven/models/fern_02/fern_02.gltf";
const STUMP: &str = "polyhaven/models/tree_stump_01/tree_stump_01.gltf";
const LOG: &str = "polyhaven/models/dead_tree_trunk/dead_tree_trunk.gltf";

/// Ferns and shrubs on the forest floor and along its edges, stumps and
/// fallen trunks deeper in. Counts are kept modest: these are detailed
/// scanned models.
fn undergrowth(w: &mut SceneBuilder, layout: &Layout) -> usize {
    let spacing = 5.5;
    let n = (2.0 * HALF / spacing) as i32;
    let mut count = 0;
    for gz in 0..n {
        for gx in 0..n {
            let jitter = Vec2::new(hash2(gx, gz, 701), hash2(gx, gz, 702));
            let p = Vec2::new(-HALF, -HALF) + (Vec2::new(gx as f32, gz as f32) + jitter) * spacing;
            let forest = layout.forest_density(p);
            let roll = hash2(gx, gz, 703);
            // Cumulative chances, densest in the woods.
            let (file, size, solid) = if roll < 0.16 * forest {
                (FERN, lerp(0.7, 1.2, hash2(gx, gz, 704)), false)
            } else if roll < 0.16 * forest + 0.07 * forest + 0.01 {
                (BUSHES[(hash2(gx, gz, 706) * BUSHES.len() as f32) as usize % BUSHES.len()], lerp(0.7, 1.3, hash2(gx, gz, 704)), false)
            } else if roll < 0.2 * forest + 0.006 + 0.012 * forest {
                (STUMP, lerp(0.45, 0.8, hash2(gx, gz, 704)), true)
            } else if roll < 0.232 * forest + 0.006 + 0.006 * forest {
                (LOG, 1.0, true)
            } else {
                continue;
            };
            if Layout::edge_distance(p) > 185.0 || w.ground(p.x, p.y) < WATER + 0.5 {
                continue;
            }
            if w.terrain.normal_at(p.x, p.y).y < 0.8 || !layout.open_ground(p, 1.2) {
                continue;
            }
            if !clear_of_colliders(&w.colliders, p, 1.0) {
                continue;
            }
            let mut placement = Placement::at(p.x, p.y).yaw(hash2(gx, gz, 705) * 360.0);
            placement = if file == LOG { placement.scale(size) } else { placement.height(size) };
            if !solid {
                placement = placement.collider(Collider::None);
            }
            if w.place(file, placement).is_some() {
                count += 1;
            }
        }
    }
    count
}

fn lantern(w: &mut SceneBuilder, x: f32, z: f32, yaw: f32) {
    let p = Placement::at(x, z)
        .height(4.5)
        .yaw(yaw)
        .collider(Collider::Box(Vec3::new(0.5, 4.5, 0.5)));
    w.place(LANTERN, p);
}

/// A lantern whose arm reaches towards `target`.
fn lantern_facing(w: &mut SceneBuilder, x: f32, z: f32, target: Vec2) {
    let d = target - Vec2::new(x, z);
    // The model's arm points along +X at yaw 0; yaw turns clockwise seen from above.
    let yaw = -d.y.atan2(d.x).to_degrees();
    lantern(w, x, z, yaw);
}

fn rock(w: &mut SceneBuilder, x: f32, z: f32, height: f32, yaw: f32, tint: Vec4) {
    let mut p = Placement::at(x, z).height(height).yaw(yaw).tint(tint);
    if height < 0.35 {
        p = p.collider(Collider::None);
    }
    // Sink rocks a little so they look bedded in.
    p = p.at_height(w.ground(x, z) - height * 0.15);
    w.place(ROCK, p);
}

fn village(w: &mut SceneBuilder) {
    let (vx, vz) = (VILLAGE.x, VILLAGE.y);
    let ground = w.ground(vx, vz);

    // The well in the middle of the square.
    // A round stone well with dark water just below the rim.
    w.round_column(Surface::StoneWall, VILLAGE, 1.3, ground, 0.9, Vec4::ONE);
    w.cylinder(
        Surface::StoneWall,
        Vec3::new(vx, ground + 0.9, vz),
        Vec3::Y,
        1.0,
        0.01,
        Vec4::new(0.1, 0.12, 0.14, 1.0),
        false,
    );
    let wood = Vec4::new(0.8, 0.68, 0.55, 1.0);
    for s in [-1.0, 1.0] {
        w.column(Surface::Planks, Vec2::new(vx + s * 1.1, vz), Vec2::splat(0.2), ground, 2.5, wood, BoxUv::World(1.0));
    }
    w.gable_roof(VILLAGE, Vec2::new(1.5, 1.0), true, ground + 2.5, 0.7, 0.15, Vec4::ONE, Surface::Planks, wood);

    // Four houses around the square, doors facing it.
    let houses = [
        House {
            center: Vec2::new(12.0, 25.0),
            size: Vec2::new(8.0, 7.0),
            door: Side::South,
            wall: Surface::Brick,
            wall_tint: Vec4::ONE,
            wall_height: 3.4,
            roof_rise: 2.2,
            roof_tint: Vec4::ONE,
        },
        House {
            center: Vec2::new(21.0, 38.0),
            size: Vec2::new(6.0, 7.0),
            door: Side::West,
            wall: Surface::Planks,
            wall_tint: Vec4::new(0.85, 0.75, 0.65, 1.0),
            wall_height: 3.0,
            roof_rise: 2.0,
            roof_tint: Vec4::new(0.7, 0.78, 0.9, 1.0),
        },
        House {
            center: Vec2::new(13.0, 57.0),
            size: Vec2::new(8.0, 7.0),
            door: Side::North,
            wall: Surface::StoneWall,
            wall_tint: Vec4::ONE,
            wall_height: 3.2,
            roof_rise: 2.3,
            roof_tint: Vec4::new(0.95, 0.85, 0.8, 1.0),
        },
        House {
            center: Vec2::new(-9.0, 58.0),
            size: Vec2::new(7.0, 6.0),
            door: Side::North,
            wall: Surface::Sandstone,
            wall_tint: Vec4::ONE,
            wall_height: 3.0,
            roof_rise: 2.0,
            roof_tint: Vec4::new(1.1, 0.92, 0.8, 1.0),
        },
    ];
    for h in &houses {
        w.house(h);
    }

    // Market corner beside the brick house.
    let tints = [Vec4::ONE, Vec4::new(1.0, 0.85, 0.75, 1.0), Vec4::new(0.85, 0.92, 1.0, 1.0)];
    w.crate_stack(6.0, 27.0, 1.1, true, tints[0]);
    w.crate_stack(4.6, 29.2, 0.9, false, tints[1]);
    w.crate_stack(7.1, 29.6, 0.7, false, tints[2]);
    w.crate_stack(8.0, 54.5, 0.8, true, tints[1]);

    // Benches facing the well, lanterns at the corners of the square.
    w.bench(vx - 4.0, vz - 4.0, true);
    w.bench(vx + 4.0, vz + 4.5, true);
    for (x, z) in [(-11.0, 34.0), (11.0, 34.0), (-11.0, 47.5), (11.0, 47.5)] {
        lantern_facing(w, vx + x, z, VILLAGE);
    }
    // At the village exits.
    lantern_facing(w, 4.5, 22.0, Vec2::new(2.0, 10.0));
    lantern_facing(w, 24.0, 47.0, Vec2::new(30.0, 52.0));

    // Fenced flower garden, gate on the east side.
    let (lo, hi) = GARDEN;
    w.fence(&[
        Vec2::new(hi.x, lo.y + 2.5),
        Vec2::new(hi.x, lo.y),
        Vec2::new(lo.x, lo.y),
        Vec2::new(lo.x, hi.y),
        Vec2::new(hi.x, hi.y),
        Vec2::new(hi.x, lo.y + 4.5),
    ]);
}

fn pier(w: &mut SceneBuilder) {
    // Start where the shore meets the water.
    let mut start = -10.0;
    while start > -30.0 && w.ground(start, PIER_Z) > WATER + 0.45 {
        start -= 0.5;
    }
    let end = start - 22.0;
    let deck_y = (WATER + 0.75).max(w.ground(start, PIER_Z) + 0.1);
    let wood = Vec4::new(0.8, 0.7, 0.6, 1.0);
    let length = start - end + 1.0;
    w.solid(
        Surface::Planks,
        Vec3::new((start + end) * 0.5, deck_y - 0.1, PIER_Z),
        Vec3::new(length, 0.2, 2.6),
        wood,
        BoxUv::World(2.0),
    );
    // Wider landing at the end.
    w.solid(
        Surface::Planks,
        Vec3::new(end - 1.5, deck_y - 0.1, PIER_Z),
        Vec3::new(4.0, 0.2, 5.0),
        wood,
        BoxUv::World(2.0),
    );
    // Posts down into the lake bed, and rails along both sides.
    let mut x = start;
    while x > end - 3.0 {
        for s in [-1.0, 1.0] {
            let z = PIER_Z + s * 1.2;
            let bed = w.ground(x, z) - 0.3;
            let top = deck_y + 0.9;
            w.visual(
                Surface::Planks,
                Vec3::new(x, (bed + top) * 0.5, z),
                Vec3::new(0.22, top - bed, 0.22),
                wood * 0.8,
                BoxUv::World(1.0),
            );
        }
        x -= 3.0;
    }
    for s in [-1.0, 1.0] {
        let z = PIER_Z + s * 1.2;
        let rail = Vec3::new(length, 0.1, 0.1);
        let c = Vec3::new((start + end) * 0.5, deck_y + 0.85, z);
        w.visual(Surface::Planks, c, rail, wood, BoxUv::World(1.0));
        w.colliders
            .push(Aabb::from_center(Vec3::new(c.x, deck_y + 0.5, z), Vec3::new(length, 1.0, 0.2)));
    }
    lantern(w, end - 3.0, PIER_Z - 2.0, 0.0);
    w.crate_stack(end - 2.5, PIER_Z + 1.6, 0.9, true, Vec4::ONE);
    w.bench(end - 0.6, PIER_Z + 1.9, true);
}

fn bridge(w: &mut SceneBuilder) {
    let (z0, z1) = BRIDGE_ENDS;
    let deck_y = w.ground(BRIDGE_X, z0 - 2.0).max(w.ground(BRIDGE_X, z1 + 2.0)) + 0.15;
    let length = z1 - z0 + 2.0;
    let center_z = (z0 + z1) * 0.5;
    let wood = Vec4::new(0.82, 0.7, 0.58, 1.0);
    w.solid(
        Surface::Planks,
        Vec3::new(BRIDGE_X, deck_y - 0.15, center_z),
        Vec3::new(3.0, 0.3, length),
        wood,
        BoxUv::World(2.0),
    );
    // Stone piers under the deck, from the river bed up.
    for z in [center_z - 8.0, center_z, center_z + 8.0] {
        let bed = w.ground(BRIDGE_X, z) - 0.5;
        let top = deck_y - 0.3;
        if top > bed {
            w.solid(
                Surface::StoneWall,
                Vec3::new(BRIDGE_X, (bed + top) * 0.5, z),
                Vec3::new(3.4, top - bed, 1.2),
                Vec4::ONE,
                BoxUv::World(2.0),
            );
        }
    }
    // Rails with posts.
    for s in [-1.0, 1.0] {
        let x = BRIDGE_X + s * 1.4;
        let mut z = z0 - 1.0;
        while z <= z1 + 1.0 {
            w.visual(
                Surface::Planks,
                Vec3::new(x, deck_y + 0.5, z),
                Vec3::new(0.18, 1.0, 0.18),
                wood * 0.85,
                BoxUv::World(1.0),
            );
            z += 2.5;
        }
        w.visual(
            Surface::Planks,
            Vec3::new(x, deck_y + 0.95, center_z),
            Vec3::new(0.12, 0.12, length),
            wood,
            BoxUv::World(1.0),
        );
        w.colliders.push(Aabb::from_center(
            Vec3::new(x, deck_y + 0.5, center_z),
            Vec3::new(0.25, 1.0, length),
        ));
    }
    lantern(w, BRIDGE_X + 2.6, z1 + 2.5, 180.0);
    lantern(w, BRIDGE_X - 2.6, z0 - 2.5, 0.0);
}

fn tower(w: &mut SceneBuilder) {
    let (tx, tz) = (TOWER.x, TOWER.y);
    let ground = w.ground(tx, tz);
    let platform_h = 4.0;
    let top = ground + platform_h;
    // A round stone platform, stairs up from the south.
    let center = Vec2::new(tx, tz - 3.0);
    let radius = 4.0;
    w.round_column(Surface::StoneWall, center, radius, ground, platform_h, Vec4::ONE);
    let step_tint = Vec4::new(0.8, 0.82, 0.88, 1.0);
    for i in 0..10 {
        let h = 0.4 * (i + 1) as f32;
        let z = tz + 1.5 + (9 - i) as f32;
        w.column(Surface::Tiles, Vec2::new(tx, z), Vec2::new(2.0, 1.0), ground, h, step_tint, BoxUv::World(2.0));
    }
    // Crenellated parapet ring, open where the stairs arrive.
    let merlons = 18;
    for i in 0..merlons {
        let a = i as f32 / merlons as f32 * TAU;
        let d = Vec2::new(a.cos(), a.sin());
        if d.y > 0.88 {
            continue; // the stair opening (south)
        }
        let p = center + d * (radius - 0.3);
        w.solid(
            Surface::StoneWall,
            Vec3::new(p.x, top + 0.4, p.y),
            Vec3::new(0.7, 0.8, 0.7),
            Vec4::ONE,
            BoxUv::World(1.0),
        );
    }
    // A lookout arch framing the view over the lake.
    for s in [-1.0, 1.0] {
        w.solid(
            Surface::StoneWall,
            Vec3::new(tx + s * 2.4, top + 2.6, tz - 5.4),
            Vec3::new(0.8, 4.5, 0.8),
            Vec4::ONE,
            BoxUv::World(2.0),
        );
    }
    w.solid(
        Surface::Planks,
        Vec3::new(tx, top + 5.1, tz - 5.4),
        Vec3::new(6.0, 0.5, 1.2),
        Vec4::new(0.8, 0.62, 0.55, 1.0),
        BoxUv::World(2.0),
    );
    lantern(w, tx + 2.5, tz + 11.5, 180.0);
    w.crate_stack(tx - 3.5, tz + 6.5, 1.0, true, Vec4::ONE);
}

fn stone_circle(w: &mut SceneBuilder) {
    let sandstone = Vec4::new(0.98, 0.95, 0.9, 1.0);
    for i in 0..8 {
        let a = i as f32 / 8.0 * TAU + 0.2;
        let p = CIRCLE + Vec2::new(a.cos(), a.sin()) * 8.0;
        let ground = w.ground_under(p, Vec2::splat(0.8));
        let broken = i == 2 || i == 5;
        let h = if broken { 1.3 } else { 3.0 + (i % 3) as f32 * 1.6 };
        // Round columns: a wider plinth, the shaft, a capital on top.
        w.round_column(Surface::Sandstone, p, 0.85, ground, 0.3, sandstone * 0.92);
        w.round_column(Surface::Sandstone, p, 0.6, ground, h, sandstone);
        if broken {
            // The fallen drum lies on the grass beside its base, tilted a little.
            let out = Vec2::new(a.cos(), a.sin());
            let start = p + out * 1.2;
            let g = w.ground_under(start + out * 1.5, Vec2::splat(1.0));
            let axis = Vec3::new(out.x, 0.06, out.y);
            w.cylinder(Surface::Sandstone, Vec3::new(start.x, g + 0.45, start.y), axis, 0.55, 3.0, sandstone * 0.9, true);
        } else {
            w.cylinder(
                Surface::Sandstone,
                Vec3::new(p.x, ground + h, p.y),
                Vec3::Y,
                0.78,
                0.3,
                sandstone * 0.92,
                true,
            );
        }
    }
    // A standing stone at the centre.
    rock(w, CIRCLE.x, CIRCLE.y, 2.4, 20.0, Vec4::new(0.95, 0.93, 0.9, 1.0));
    lantern(w, CIRCLE.x + 7.0, CIRCLE.y + 12.0, 200.0);
}

fn fishers_rest(w: &mut SceneBuilder) {
    let (x, z) = (FISHER.x, FISHER.y);
    w.bench(x + 1.5, z, false);
    lantern_facing(w, x - 1.0, z - 3.0, LAKE_CENTER);
    w.crate_stack(x - 1.5, z + 2.5, 0.9, true, Vec4::ONE);
    rock(w, x + 2.5, z + 4.0, 1.1, 40.0, Vec4::ONE);
}

fn viewpoint(w: &mut SceneBuilder) {
    let (x, z) = (VIEWPOINT.x, VIEWPOINT.y);
    // Looking back north over the meadow and the valley.
    w.bench(x, z + 1.5, true);
    lantern(w, x - 2.5, z - 1.0, 270.0);
    rock(w, x + 3.5, z + 2.0, 1.6, 75.0, Vec4::ONE);
}

/// True if `p` is clear of everything already built (with a margin).
fn clear_of_colliders(colliders: &[Aabb], p: Vec2, margin: f32) -> bool {
    colliders.iter().all(|c| {
        p.x < c.min.x - margin || p.x > c.max.x + margin || p.y < c.min.z - margin || p.y > c.max.z + margin
    })
}

fn rocks(w: &mut SceneBuilder, layout: &Layout) {
    let spacing = 9.0;
    let n = (2.0 * HALF / spacing) as i32;
    for gz in 0..n {
        for gx in 0..n {
            let jitter = Vec2::new(hash2(gx, gz, 501), hash2(gx, gz, 502));
            let p = Vec2::new(-HALF, -HALF) + (Vec2::new(gx as f32, gz as f32) + jitter) * spacing;
            let e = Layout::edge_distance(p);
            if e > 188.0 {
                continue;
            }
            let lake = layout.lake_distance(p);
            let dr = layout.river.at(p);
            let slope = 1.0 - w.terrain.normal_at(p.x, p.y).y;
            let mountain_foot = smoothstep(140.0, 165.0, e) * 0.3;
            let shore = if (0.95..1.18).contains(&lake) { 0.35 } else { 0.0 };
            let bank = if (4.0..10.0).contains(&dr) { 0.3 } else { 0.0 };
            let hillside = smoothstep(0.15, 0.3, slope) * 0.3;
            let chance = mountain_foot.max(shore).max(bank).max(hillside).max(0.03);
            if hash2(gx, gz, 503) > chance {
                continue;
            }
            if !layout.open_ground(p, 1.5) || !clear_of_colliders(&w.colliders, p, 2.0) {
                continue;
            }
            // Only on ground gentle enough for a rock to rest on.
            if slope > 0.3 || w.ground(p.x, p.y) < WATER - 0.4 {
                continue;
            }
            let size = if mountain_foot > 0.2 {
                lerp(1.5, 4.5, hash2(gx, gz, 504))
            } else {
                lerp(0.2, 1.4, hash2(gx, gz, 504).powi(2))
            };
            let warm = lerp(0.9, 1.1, hash2(gx, gz, 505));
            let yaw = hash2(gx, gz, 506) * 360.0;
            // Big ones are photo-scanned boulders (if downloaded); the rest
            // are the low-poly rock.
            let scanned = if size > 1.3 {
                if hash2(gx, gz, 511) < 0.5 { Some(BOULDER) } else { Some(MOSS_ROCKS) }
            } else if size > 0.6 && hash2(gx, gz, 512) < 0.4 {
                Some(SMALL_ROCK)
            } else {
                None
            };
            let placed = scanned.and_then(|file| {
                let g = w.ground(p.x, p.y) - size * 0.12;
                w.place(file, Placement::at(p.x, p.y).height(size).yaw(yaw).at_height(g))
            });
            if placed.is_none() {
                rock(w, p.x, p.y, size, yaw, Vec4::new(warm, 1.0, 2.0 - warm, 1.0));
            }
            // Sometimes a smaller companion stone.
            if hash2(gx, gz, 507) < 0.35 {
                let q = p + Vec2::new(hash2(gx, gz, 508) - 0.5, hash2(gx, gz, 509) - 0.5) * 3.0;
                rock(w, q.x, q.y, size * 0.4, hash2(gx, gz, 510) * 360.0, Vec4::ONE);
            }
        }
    }
}

fn trees(w: &mut SceneBuilder, layout: &Layout) -> usize {
    // Spaced for full-size trees, so you can walk between them and light
    // reaches the forest floor.
    let spacing = 6.0;
    let n = (2.0 * HALF / spacing) as i32;
    let mut count = 0;
    for gz in 0..n {
        for gx in 0..n {
            let jitter = Vec2::new(hash2(gx, gz, 601), hash2(gx, gz, 602));
            let p = Vec2::new(-HALF, -HALF) + (Vec2::new(gx as f32, gz as f32) + jitter) * spacing;
            let density = layout.forest_density(p);
            // Lone trees dot the open land; more of them line the water.
            let shore = layout.lake_distance(p) < 1.45 || layout.river.at(p) < 15.0;
            let lone = if shore { 0.06 } else { 0.014 };
            if hash2(gx, gz, 603) > density.max(lone) {
                continue;
            }
            if Layout::edge_distance(p) > 186.0 || w.ground(p.x, p.y) < WATER + 0.6 {
                continue;
            }
            if w.terrain.normal_at(p.x, p.y).y < 0.8 {
                continue;
            }
            if !layout.open_ground(p, 2.0) || !clear_of_colliders(&w.colliders, p, 2.5) {
                continue;
            }
            // Bigger, darker trees deep in the forest.
            let depth = density.clamp(0.0, 1.0);
            let s = lerp(0.85, 1.3, hash2(gx, gz, 604)) + depth * 0.3;
            let shade = lerp(1.0, 0.85, depth);
            let ground = w.ground(p.x, p.y);
            let species = species_at(p, density < 0.05, ground, hash2(gx, gz, 609), hash2(gx, gz, 610));
            // Autumn colours shouldn't be tinted green; keep their variation subtle.
            let spread = if COMMON_AUTUMN.contains(&species.file) || PINES_AUTUMN.contains(&species.file) { 0.06 } else { 0.12 };
            let tint = Vec4::new(
                lerp(1.0 - spread, 1.0 + spread * 0.7, hash2(gx, gz, 605)) * shade,
                lerp(1.0 - spread, 1.0 + spread * 0.8, hash2(gx, gz, 606)) * shade,
                lerp(1.0 - spread * 1.5, 1.0, hash2(gx, gz, 607)) * shade,
                1.0,
            );
            let trunk = species.trunk * s;
            let tree = Placement::at(p.x, p.y)
                .height(species.height * s)
                .yaw(hash2(gx, gz, 608) * 360.0)
                .tint(tint)
                // The trunk is solid all the way up, so there's no invisible
                // ledge to land on inside the canopy.
                .collider(Collider::Box(Vec3::new(trunk, species.height * s, trunk)));
            if w.place(species.file, tree).is_none() {
                continue; // that species' model is missing; error already logged
            }
            count += 1;
        }
    }
    count
}

/// Where the 15 skill crystals float (one per ability level). Some are at
/// landmarks, some hidden, and some need abilities to reach.
fn crystal_spots(w: &SceneBuilder) -> Vec<Vec3> {
    let on_ground = |x: f32, z: f32| Vec3::new(x, w.ground(x, z) + 1.2, z);
    let mut pier_start = -10.0;
    while pier_start > -30.0 && w.ground(pier_start, PIER_Z) > WATER + 0.45 {
        pier_start -= 0.5;
    }
    let tower_top = w.ground(TOWER.x, TOWER.y) + 4.0;
    let bridge_deck = w.ground(BRIDGE_X, BRIDGE_ENDS.0 - 2.0).max(w.ground(BRIDGE_X, BRIDGE_ENDS.1 + 2.0)) + 0.15;
    let roof_ridge = w.ground_under(Vec2::new(12.0, 25.0), Vec2::new(4.0, 3.5)) + 0.05 + 3.4 + 2.2;
    vec![
        // Easy finds near the start.
        on_ground(VILLAGE.x + 3.0, VILLAGE.y - 2.0),
        Vec3::new(pier_start - 22.5, WATER + 1.9, PIER_Z),
        Vec3::new(BRIDGE_X, bridge_deck + 1.2, (BRIDGE_ENDS.0 + BRIDGE_ENDS.1) * 0.5),
        on_ground(FISHER.x + 1.0, FISHER.y + 1.0),
        on_ground(VIEWPOINT.x, VIEWPOINT.y - 2.5),
        // Exploring further out.
        on_ground(-40.0, -126.0),
        on_ground(138.0, -35.0),
        on_ground(-120.0, 90.0),
        on_ground(126.0, -150.0),
        Vec3::new(TOWER.x, tower_top + 1.2, TOWER.y - 3.0),
        // Needs abilities: high jump / double jump / gliding.
        Vec3::new(CIRCLE.x, w.ground(CIRCLE.x, CIRCLE.y) + 3.6, CIRCLE.y),
        Vec3::new(12.0, roof_ridge + 0.8, 25.0),
        Vec3::new(TOWER.x, tower_top + 6.4, TOWER.y - 5.4),
        Vec3::new(LAKE_CENTER.x, WATER + 2.5, LAKE_CENTER.y),
        on_ground(0.0, -178.0),
    ]
}

pub fn build() -> Scene {    let started = Instant::now();
    let mut textures: HashMap<Surface, SurfaceTextures> = textures::generate_all().into_iter().collect();
    let materials: HashMap<Surface, MaterialData> = textures
        .iter()
        .map(|(&s, t)| (s, surface_material(s, t)))
        .collect();

    let layout = Layout::new();
    let terrain = Terrain::from_fn(HALF, 1.0, WATER, |x, z| layout.height(Vec2::new(x, z)));
    log::info!("terrain shaped in {:.2?}", started.elapsed());

    let mut w = SceneBuilder::new(terrain);
    village(&mut w);
    pier(&mut w);
    bridge(&mut w);
    tower(&mut w);
    stone_circle(&mut w);
    fishers_rest(&mut w);
    viewpoint(&mut w);
    rocks(&mut w, &layout);
    let tree_count = trees(&mut w, &layout);
    let plants = undergrowth(&mut w, &layout);
    log::info!("placed {tree_count} trees, {plants} plants/logs, {} colliders", w.colliders.len());

    let crystals = crystal_spots(&w);
    let splat = layout.splat(&w.terrain);
    let mask = layout.grass_mask(&w.terrain, &w.colliders);
    log::info!("ground textures built in {:.2?}", started.elapsed());

    // South-east corner of the square, looking across the well to the lake.
    let spawn_xz = VILLAGE + Vec2::new(9.0, 9.0);
    let spawn = Vec3::new(spawn_xz.x, w.ground(spawn_xz.x, spawn_xz.y), spawn_xz.y);
    let to_lake = LAKE_CENTER - spawn_xz;
    let (terrain, static_parts, models, colliders) = w.finish(&materials);
    let height_range = terrain.min_max();

    let mut take = |s: Surface| textures.remove(&s).expect("ground texture");
    let ground_layers = [
        take(Surface::Grass),
        take(Surface::Dirt),
        take(Surface::Rock),
        take(Surface::Sand),
        take(Surface::Cobble),
    ];
    let grass = GrassField {
        mask,
        albedo: ground_layers[0].albedo.clone(),
        half_size: HALF,
        macro_variation: 0.35,
        spacing: 0.1,
        radius: 26.0,
        min_height: 0.22,
        max_height: 0.5,
        blade_width: 0.045,
        wind: 0.35,
    };

    Scene {
        crystals,
        static_parts,
        grass: Some(grass),
        models,
        colliders,
        terrain,
        ground_layers,
        ground_splat: splat,
        spawn,
        // Looking out across the lake towards the tower.
        spawn_yaw: to_lake.y.atan2(to_lake.x),
        lighting: Lighting {
            // Late-morning sun (the settings menu overrides these).
            sun_direction: Vec3::new(0.55, -0.62, 0.58),
            sun_color: Vec3::new(1.0, 0.92, 0.8) * 2.7,
            sky_zenith: Vec3::new(0.16, 0.36, 0.85),
            sky_horizon: Vec3::new(0.70, 0.80, 0.92),
            sky_ambient: Vec3::new(0.40, 0.50, 0.70),
            ground_ambient: Vec3::new(0.22, 0.20, 0.14),
            fog_start: 60.0,
            fog_end: 320.0,
            exposure: 0.9,
            shadow_near_radius: 45.0,
            world_half_size: HALF,
            world_height_range: height_range,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_and_crystals_are_clear_of_colliders() {
        let scene = build();
        let p = scene.spawn;
        let body = Aabb {
            min: p + Vec3::new(-0.3, 0.05, -0.3),
            max: p + Vec3::new(0.3, 1.8, 0.3),
        };
        assert!(!scene.colliders.iter().any(|c| c.intersects(&body)), "spawn is inside something");
        for (i, &c) in scene.crystals.iter().enumerate() {
            let probe = Aabb::from_center(c, Vec3::splat(0.4));
            assert!(!scene.colliders.iter().any(|b| b.intersects(&probe)), "crystal {i} at {c} is inside something");
        }
    }

    #[test]
    fn landmarks_sit_where_they_should() {
        let l = Layout::new();
        assert!(l.height(VILLAGE) > WATER + 2.5, "village must be well above the lake");
        assert!(l.height(LAKE_CENTER) < WATER - 3.5, "lake must be deep in the middle");
        assert!(l.height(Vec2::new(BRIDGE_X, -55.0)) < WATER - 0.5, "river under the bridge");
        assert!(l.height(TOWER) > l.height(LAKE_CENTER + Vec2::new(0.0, -30.0)) + 10.0, "tower on high ground");
        assert!(l.height(CIRCLE) > l.height(VILLAGE) + 8.0, "stone circle on a hill");
        assert!(l.height(Vec2::new(0.0, 195.0)) > 40.0, "mountains at the edge");
        // The pier reaches deep enough water.
        assert!(l.height(Vec2::new(-36.0, PIER_Z)) < WATER - 0.8);
    }

    #[test]
    fn paths_stay_on_dry_land() {
        let l = Layout::new();
        for (i, (points, _)) in path_points().iter().enumerate() {
            for p in &spline(points, 1.0) {
                let h = l.height(*p);
                assert!(h > WATER + 0.15, "path {i} dips into water at {p:?} (height {h})");
            }
        }
    }

    #[test]
    fn bridge_banks_are_level() {
        let l = Layout::new();
        let a = l.height(Vec2::new(BRIDGE_X, BRIDGE_ENDS.0 - 2.0));
        let b = l.height(Vec2::new(BRIDGE_X, BRIDGE_ENDS.1 + 2.0));
        assert!((a - b).abs() < 0.05, "banks at {a} and {b}");
    }
}
