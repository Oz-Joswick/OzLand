//! Procedurally generated material textures: an sRGB albedo map plus a
//! tangent-space normal map derived from a height field. All except the
//! crate are seamless, so they can repeat across large surfaces.

use std::sync::Arc;

use glam::{Vec2, Vec3};

use crate::model::TextureData;

const SIZE: u32 = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Surface {
    Grass,
    StoneWall,
    Brick,
    Tiles,
    Crate,
    Planks,
    Shingles,
    Cobble,
    Sandstone,
    Dirt,
    Rock,
    Sand,
}

impl Surface {
    pub const ALL: [Surface; 12] = [
        Surface::Grass,
        Surface::StoneWall,
        Surface::Brick,
        Surface::Tiles,
        Surface::Crate,
        Surface::Planks,
        Surface::Shingles,
        Surface::Cobble,
        Surface::Sandstone,
        Surface::Dirt,
        Surface::Rock,
        Surface::Sand,
    ];
}

pub struct SurfaceTextures {
    pub albedo: Arc<TextureData>,
    /// Tangent-space normal in RGB, height (0 = deepest, 1 = top) in alpha.
    pub normal: Arc<TextureData>,
}

/// One texel: albedo in sRGB 0..1 and a height in 0..1.
struct Texel {
    color: Vec3,
    height: f32,
}

/// Generates every surface's textures in parallel.
pub fn generate_all() -> Vec<(Surface, SurfaceTextures)> {
    std::thread::scope(|s| {
        let handles: Vec<_> = Surface::ALL
            .iter()
            .map(|&surface| s.spawn(move || (surface, generate(surface))))
            .collect();
        handles.into_iter().map(|h| h.join().expect("texture thread panicked")).collect()
    })
}

/// A photo-scanned texture set from `assets/polyhaven/textures/<folder>`
/// (fetched by `tools/fetch_assets.ps1`), if it has been downloaded:
/// diffuse, OpenGL-style normal map and height map, resized to SIZE.
fn scanned(surface: Surface) -> Option<SurfaceTextures> {
    let folder = match surface {
        Surface::Dirt => "dirt",
        Surface::Rock => "rock",
        Surface::Sand => "sand",
        Surface::Cobble => "cobble",
        Surface::Brick => "brick",
        Surface::StoneWall => "stonewall",
        Surface::Planks => "planks",
        Surface::Shingles => "shingles",
        Surface::Sandstone => "sandstone",
        _ => return None,
    };
    let dir = crate::model::asset_root().join("polyhaven/textures").join(folder);
    let load = |file: &str| {
        image::open(dir.join(file))
            .ok()
            .map(|img| img.resize_exact(SIZE, SIZE, image::imageops::FilterType::Triangle))
    };
    let albedo = load("diffuse.jpg")?.to_rgba8();
    let normal = load("normal.jpg")?.to_rgb8();
    let height = load("height.png")?.to_luma8();
    let mut normal_height = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for (n, h) in normal.pixels().zip(height.pixels()) {
        normal_height.extend_from_slice(&[n[0], n[1], n[2], h[0]]);
    }
    log::info!("using scanned texture for {surface:?}");
    Some(SurfaceTextures {
        albedo: Arc::new(TextureData { width: SIZE, height: SIZE, rgba: albedo.into_raw(), srgb: true }),
        normal: Arc::new(TextureData { width: SIZE, height: SIZE, rgba: normal_height, srgb: false }),
    })
}

/// The scanned texture set when available, otherwise a generated one.
pub fn generate(surface: Surface) -> SurfaceTextures {
    if let Some(t) = scanned(surface) {
        return t;
    }
    match surface {
        Surface::Grass => build(1.6, grass),
        Surface::StoneWall => build(5.0, |uv| {
            bricks(uv, &BrickStyle {
                rows: 4,
                cols: 2,
                staggered: true,
                mortar: 0.010,
                bevel: 0.025,
                colors: [Vec3::new(0.55, 0.53, 0.50), Vec3::new(0.66, 0.64, 0.60), Vec3::new(0.47, 0.47, 0.46)],
                mortar_color: Vec3::new(0.36, 0.35, 0.33),
                seed: 11,
            })
        }),
        Surface::Brick => build(4.0, |uv| {
            bricks(uv, &BrickStyle {
                rows: 28,
                cols: 8,
                staggered: true,
                mortar: 0.0035,
                bevel: 0.006,
                colors: [Vec3::new(0.62, 0.29, 0.20), Vec3::new(0.72, 0.38, 0.25), Vec3::new(0.50, 0.24, 0.18)],
                mortar_color: Vec3::new(0.74, 0.71, 0.65),
                seed: 23,
            })
        }),
        Surface::Tiles => build(4.0, |uv| {
            bricks(uv, &BrickStyle {
                rows: 2,
                cols: 2,
                staggered: false,
                mortar: 0.008,
                bevel: 0.03,
                colors: [Vec3::new(0.80, 0.75, 0.64), Vec3::new(0.74, 0.68, 0.57), Vec3::new(0.85, 0.81, 0.71)],
                mortar_color: Vec3::new(0.46, 0.43, 0.39),
                seed: 37,
            })
        }),
        Surface::Crate => build(5.0, crate_face),
        Surface::Planks => build(4.0, |uv| planks(uv, 6, 51)),
        Surface::Shingles => build(5.0, shingles),
        Surface::Cobble => build(5.0, cobble),
        Surface::Sandstone => build(3.0, |uv| {
            bricks(uv, &BrickStyle {
                rows: 8,
                cols: 3,
                staggered: true,
                mortar: 0.0025,
                bevel: 0.012,
                colors: [Vec3::new(0.80, 0.71, 0.55), Vec3::new(0.74, 0.64, 0.49), Vec3::new(0.85, 0.77, 0.62)],
                mortar_color: Vec3::new(0.62, 0.55, 0.44),
                seed: 83,
            })
        }),
        Surface::Dirt => build(3.0, dirt),
        Surface::Rock => build(6.0, rock),
        Surface::Sand => build(2.0, sand),
    }
}

/// Fills a SIZE x SIZE texture from `f(uv)` and derives the normal map.
fn build(normal_strength: f32, f: impl Fn(Vec2) -> Texel) -> SurfaceTextures {
    let n = (SIZE * SIZE) as usize;
    let mut albedo = Vec::with_capacity(n * 4);
    let mut heights = Vec::with_capacity(n);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let uv = Vec2::new((x as f32 + 0.5) / SIZE as f32, (y as f32 + 0.5) / SIZE as f32);
            let t = f(uv);
            let c = (t.color.clamp(Vec3::ZERO, Vec3::ONE) * 255.0).round();
            albedo.extend_from_slice(&[c.x as u8, c.y as u8, c.z as u8, 255]);
            heights.push(t.height);
        }
    }

    // Central differences (wrapping, so seamless textures stay seamless).
    // Tangent-space Y points up the image (glTF/OpenGL convention), i.e.
    // towards decreasing rows, hence +dy below.
    let s = SIZE as i32;
    let h = |x: i32, y: i32| heights[(y.rem_euclid(s) * s + x.rem_euclid(s)) as usize];
    let scale = normal_strength * SIZE as f32 / 512.0;
    // The height itself goes in the alpha channel, stretched to the full
    // 0..1 range, for parallax occlusion mapping (1 = top of the surface).
    let (lo, hi) = heights
        .iter()
        .fold((f32::MAX, f32::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    let range = (hi - lo).max(1e-6);
    let mut normal = Vec::with_capacity(n * 4);
    for y in 0..s {
        for x in 0..s {
            let dx = (h(x + 1, y) - h(x - 1, y)) * 0.5 * scale;
            let dy = (h(x, y + 1) - h(x, y - 1)) * 0.5 * scale;
            let v = Vec3::new(-dx, dy, 1.0).normalize() * 0.5 + 0.5;
            let v = (v * 255.0).round();
            let height = ((h(x, y) - lo) / range * 255.0).round();
            normal.extend_from_slice(&[v.x as u8, v.y as u8, v.z as u8, height as u8]);
        }
    }

    SurfaceTextures {
        albedo: Arc::new(TextureData { width: SIZE, height: SIZE, rgba: albedo, srgb: true }),
        normal: Arc::new(TextureData { width: SIZE, height: SIZE, rgba: normal, srgb: false }),
    }
}

// ---------------------------------------------------------------------------
// Noise
// ---------------------------------------------------------------------------

fn hash2(x: i32, y: i32, seed: u32) -> f32 {
    let mut h = seed
        .wrapping_mul(0x9E37_79B9)
        ^ (x as u32).wrapping_mul(0x27D4_EB2D)
        ^ (y as u32).wrapping_mul(0x1656_67B1);
    h = (h ^ (h >> 15)).wrapping_mul(0x85EB_CA6B);
    h = (h ^ (h >> 13)).wrapping_mul(0xC2B2_AE35);
    h ^= h >> 16;
    (h >> 8) as f32 / 16_777_216.0
}

fn lerp<T: std::ops::Add<Output = T> + std::ops::Sub<Output = T> + std::ops::Mul<f32, Output = T> + Copy>(
    a: T,
    b: T,
    t: f32,
) -> T {
    a + (b - a) * t
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Value noise over `uv` in 0..1 with `cells` lattice cells per axis; tiles
/// seamlessly at the texture edges.
fn vnoise(uv: Vec2, cells: (i32, i32), seed: u32) -> f32 {
    let p = uv * Vec2::new(cells.0 as f32, cells.1 as f32);
    let i = p.floor();
    let f = p - i;
    let u = f * f * (Vec2::splat(3.0) - 2.0 * f);
    let (x0, y0) = (i.x as i32, i.y as i32);
    let at = |dx: i32, dy: i32| hash2((x0 + dx).rem_euclid(cells.0), (y0 + dy).rem_euclid(cells.1), seed);
    lerp(lerp(at(0, 0), at(1, 0), u.x), lerp(at(0, 1), at(1, 1), u.x), u.y)
}

/// Fractal (multi-octave) value noise, 0..1, seamless.
fn fbm(uv: Vec2, cells: (i32, i32), octaves: u32, seed: u32) -> f32 {
    let (mut sum, mut amp, mut norm) = (0.0, 0.5, 0.0);
    let mut c = cells;
    for o in 0..octaves {
        sum += amp * vnoise(uv, c, seed.wrapping_add(o * 1013));
        norm += amp;
        amp *= 0.5;
        c = (c.0 * 2, c.1 * 2);
    }
    sum / norm
}

/// Seamless Voronoi: distances to the nearest and second-nearest jittered
/// point (in cell units), and a random value for the nearest cell.
fn voronoi(uv: Vec2, cells: i32, seed: u32) -> (f32, f32, f32) {
    let p = uv * cells as f32;
    let i = p.floor();
    let (mut f1, mut f2, mut id) = (f32::MAX, f32::MAX, 0.0);
    for dy in -1..=1 {
        for dx in -1..=1 {
            let c = Vec2::new(i.x + dx as f32, i.y + dy as f32);
            let (wx, wy) = ((c.x as i32).rem_euclid(cells), (c.y as i32).rem_euclid(cells));
            let jitter = Vec2::new(hash2(wx, wy, seed), hash2(wx, wy, seed + 1)) * 0.8 + 0.1;
            let d = (c + jitter - p).length();
            if d < f1 {
                f2 = f1;
                f1 = d;
                id = hash2(wx, wy, seed + 2);
            } else if d < f2 {
                f2 = d;
            }
        }
    }
    (f1, f2, id)
}

// ---------------------------------------------------------------------------
// Patterns
// ---------------------------------------------------------------------------

fn grass(uv: Vec2) -> Texel {
    // Fine-grained only: large-scale variation comes from world-space noise
    // in the terrain shader, so the 4 m repeat doesn't show from afar.
    let base = fbm(uv, (8, 8), 4, 1);
    let patches = fbm(uv, (6, 6), 3, 7);
    let fine = vnoise(uv, (256, 256), 3);
    let tufts = fbm(uv, (48, 48), 3, 5);
    let clumps = fbm(uv, (12, 12), 4, 9);
    let dark = Vec3::new(0.13, 0.21, 0.07);
    let light = Vec3::new(0.29, 0.38, 0.13);
    let dry = Vec3::new(0.43, 0.40, 0.21);
    let mut color = lerp(dark, light, base);
    color = lerp(color, dry, smoothstep(0.58, 0.8, patches) * 0.35);
    // Darker, denser clumps and fine blade-scale speckle.
    color *= 0.75 + 0.4 * smoothstep(0.3, 0.7, clumps);
    let detail = fine * 0.5 + tufts * 0.5;
    color *= 0.62 + 0.7 * detail;
    Texel { color, height: detail * 0.7 + clumps * 0.3 }
}

struct BrickStyle {
    rows: i32,
    cols: i32,
    staggered: bool,
    /// Mortar half-width and edge bevel, in UV units.
    mortar: f32,
    bevel: f32,
    colors: [Vec3; 3],
    mortar_color: Vec3,
    seed: u32,
}

/// Bricks, stone blocks or floor tiles laid in rows.
fn bricks(uv: Vec2, s: &BrickStyle) -> Texel {
    let row = (uv.y * s.rows as f32).floor();
    let fy = uv.y * s.rows as f32 - row;
    let shift = if s.staggered && (row as i32) % 2 == 1 { 0.5 } else { 0.0 };
    let x = uv.x * s.cols as f32 + shift;
    let col = x.floor();
    let fx = x - col;
    let id = hash2((col as i32).rem_euclid(s.cols), row as i32, s.seed);

    let edge = (fx.min(1.0 - fx) / s.cols as f32).min(fy.min(1.0 - fy) / s.rows as f32);
    let wobble = fbm(uv, (16, 16), 4, s.seed + 5);
    let edge = edge + (wobble - 0.5) * s.mortar * 1.2;
    let solid = smoothstep(s.mortar, s.mortar + s.bevel, edge);

    let mut base = lerp(s.colors[0], s.colors[1], id);
    if hash2((col as i32).rem_euclid(s.cols), row as i32, s.seed + 3) > 0.8 {
        base = s.colors[2];
    }
    let grain = fbm(uv, (32, 32), 4, s.seed + 9);
    let speck = vnoise(uv, (256, 256), s.seed + 13);
    let brick = base * (0.75 + 0.35 * grain + 0.12 * (speck - 0.5));
    let mortar = s.mortar_color * (0.85 + 0.3 * wobble);
    Texel {
        color: lerp(mortar, brick, solid),
        height: solid * (0.75 + 0.25 * grain) + (1.0 - solid) * 0.1 * speck,
    }
}

/// Horizontal wooden planks with grain.
fn planks(uv: Vec2, rows: i32, seed: u32) -> Texel {
    let row = (uv.y * rows as f32).floor();
    let fy = uv.y * rows as f32 - row;
    let x = uv.x * 2.0 + hash2(row as i32, 0, seed);
    let col = x.floor();
    let fx = x - col;
    let id = hash2((col as i32).rem_euclid(2), row as i32, seed + 1);

    let edge = (fx.min(1.0 - fx) / 2.0).min(fy.min(1.0 - fy) / rows as f32);
    let solid = smoothstep(0.0015, 0.006, edge);
    let grain = fbm(uv + Vec2::new(id * 0.37, 0.0), (2, 64), 4, seed + 2);
    let streak = 0.5 + 0.5 * (grain * 30.0 + id * 6.0).sin();
    let wood = lerp(Vec3::new(0.42, 0.27, 0.15), Vec3::new(0.62, 0.44, 0.26), id * 0.6 + grain * 0.4);
    let wood = wood * (0.82 + 0.22 * streak);
    Texel {
        color: lerp(Vec3::new(0.12, 0.08, 0.05), wood, solid),
        height: solid * (0.8 + 0.2 * streak),
    }
}

/// One crate side: a thick frame, a diagonal brace and planks behind.
fn crate_face(uv: Vec2) -> Texel {
    let border = 0.11;
    let edge_dist = uv.x.min(1.0 - uv.x).min(uv.y.min(1.0 - uv.y));
    let frame = smoothstep(border, border - 0.01, edge_dist);
    let brace_dist = (uv.x - uv.y).abs() / std::f32::consts::SQRT_2;
    let brace = smoothstep(0.065, 0.055, brace_dist) * (1.0 - frame);

    let back = planks(uv, 4, 77);
    let grain = fbm(uv, (2, 32), 4, 90);
    let frame_wood = lerp(Vec3::new(0.38, 0.24, 0.13), Vec3::new(0.5, 0.33, 0.18), grain);

    // Nails in the frame corners.
    let corner = Vec2::new(uv.x.min(1.0 - uv.x), uv.y.min(1.0 - uv.y));
    let nail = smoothstep(0.022, 0.012, (corner - Vec2::splat(border * 0.5)).length());

    let top = (frame + brace).min(1.0);
    let mut color = lerp(back.color * 0.8, frame_wood, top);
    color = lerp(color, Vec3::new(0.22, 0.22, 0.24), nail);
    // Bevel the frame's inner edge.
    let bevel = smoothstep(border - 0.02, border - 0.005, edge_dist) * frame;
    Texel {
        color,
        height: back.height * 0.45 + top * 0.55 - bevel * 0.15 + nail * 0.1,
    }
}

/// Overlapping clay roof shingles.
fn shingles(uv: Vec2) -> Texel {
    let rows = 12;
    let row = (uv.y * rows as f32).floor();
    let fy = uv.y * rows as f32 - row;
    let shift = if (row as i32) % 2 == 1 { 0.5 } else { 0.0 };
    let x = uv.x * 8.0 + shift;
    let col = x.floor();
    let fx = x - col;
    let id = hash2((col as i32).rem_euclid(8), row as i32, 61);

    let gap = smoothstep(0.0, 0.05, fx.min(1.0 - fx));
    let base = lerp(Vec3::new(0.48, 0.24, 0.17), Vec3::new(0.60, 0.32, 0.21), id);
    let base = if hash2((col as i32).rem_euclid(8), row as i32, 62) > 0.85 {
        Vec3::new(0.40, 0.22, 0.17)
    } else {
        base
    };
    let grain = fbm(uv, (16, 16), 4, 63);
    // Each shingle thickens towards its lower edge and is shadowed at the
    // top where the row above overlaps it.
    let shade = 0.55 + 0.45 * smoothstep(0.0, 0.35, fy);
    Texel {
        color: base * shade * (0.8 + 0.35 * grain) * (0.4 + 0.6 * gap),
        height: (0.2 + 0.8 * fy) * gap,
    }
}

/// Packed earth with scattered pebbles (paths, forest floor, garden).
fn dirt(uv: Vec2) -> Texel {
    let base = fbm(uv, (6, 6), 5, 101);
    let fine = vnoise(uv, (128, 128), 102);
    let (f1, _, id) = voronoi(uv, 28, 103);
    // Only some Voronoi cells hold a pebble.
    let pebble = smoothstep(0.2, 0.08, f1 + (fine - 0.5) * 0.12) * smoothstep(0.8, 0.84, id);
    let earth = lerp(Vec3::new(0.27, 0.20, 0.13), Vec3::new(0.43, 0.33, 0.22), base) * (0.82 + 0.3 * fine);
    let stone = Vec3::new(0.40, 0.37, 0.33) * (0.8 + 0.3 * id);
    Texel {
        color: lerp(earth, stone, pebble),
        height: base * 0.35 + fine * 0.15 + pebble * 0.6,
    }
}

/// Layered, cracked stone with patches of moss (cliffs, mountains).
fn rock(uv: Vec2) -> Texel {
    let n = fbm(uv, (4, 4), 6, 201);
    let ridges = 1.0 - (fbm(uv, (8, 8), 4, 202) * 2.0 - 1.0).abs();
    let cracks = smoothstep(0.05, 0.0, (fbm(uv, (6, 6), 4, 203) - 0.5).abs());
    let strata = ((uv.y * 6.0 + n * 1.5) * std::f32::consts::TAU).sin() * 0.5 + 0.5;
    let mut color = lerp(
        Vec3::new(0.24, 0.23, 0.22),
        Vec3::new(0.45, 0.42, 0.38),
        n * 0.6 + strata * 0.2 + ridges * 0.2,
    );
    let moss = smoothstep(0.6, 0.75, fbm(uv, (3, 3), 4, 204)) * 0.55;
    color = lerp(color, Vec3::new(0.28, 0.34, 0.17), moss);
    color *= 1.0 - cracks * 0.55;
    Texel {
        color,
        height: n * 0.55 + ridges * 0.3 + strata * 0.15 - cracks * 0.35,
    }
}

/// Fine sand with wind ripples (beaches, river and lake beds).
fn sand(uv: Vec2) -> Texel {
    let warp = fbm(uv, (3, 3), 3, 301);
    let ripples = ((uv.y * 10.0 + warp * 1.5) * std::f32::consts::TAU).sin() * 0.5 + 0.5;
    let grain = vnoise(uv, (256, 256), 302);
    let base = fbm(uv, (4, 4), 4, 303);
    let color = lerp(Vec3::new(0.66, 0.58, 0.43), Vec3::new(0.79, 0.72, 0.55), base)
        * (0.9 + 0.15 * grain)
        * (0.95 + 0.08 * ripples);
    Texel {
        color,
        height: ripples * 0.5 + grain * 0.3 + base * 0.2,
    }
}

/// Rounded cobblestones from Voronoi cells.
fn cobble(uv: Vec2) -> Texel {
    let (f1, f2, id) = voronoi(uv, 10, 71);
    let edge = f2 - f1;
    let stone = smoothstep(0.03, 0.16, edge);
    let dome = (1.0 - f1 * 1.2).max(0.0);
    let grain = fbm(uv, (24, 24), 4, 72);
    let base = lerp(Vec3::new(0.34, 0.33, 0.31), Vec3::new(0.48, 0.46, 0.42), id);
    let base = lerp(base, Vec3::new(0.47, 0.41, 0.33), (id * 7.0).fract() * 0.5);
    let gap_color = Vec3::new(0.19, 0.17, 0.13) * (0.8 + 0.4 * grain);
    Texel {
        color: lerp(gap_color, base * (0.75 + 0.35 * grain), stone),
        height: stone * (0.6 + 0.4 * dome) + grain * 0.08,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn textures_have_expected_size_and_unit_normals() {
        // The crate is always generated (never replaced by a scanned texture).
        let t = generate(Surface::Crate);
        assert_eq!(t.albedo.rgba.len(), (SIZE * SIZE * 4) as usize);
        assert!(t.albedo.srgb && !t.normal.srgb);
        for px in t.normal.rgba.chunks_exact(4).step_by(97) {
            let n = Vec3::new(px[0] as f32, px[1] as f32, px[2] as f32) / 127.5 - 1.0;
            assert!((n.length() - 1.0).abs() < 0.03, "normal {n:?}");
            assert!(n.z > 0.0, "normals must face out of the surface");
        }
    }

    #[test]
    fn noise_tiles_seamlessly() {
        for i in 0..50 {
            let y = i as f32 / 50.0;
            let left = fbm(Vec2::new(0.0, y), (4, 4), 4, 1);
            let right = fbm(Vec2::new(1.0, y), (4, 4), 4, 1);
            assert!((left - right).abs() < 1e-5);
        }
    }
}
