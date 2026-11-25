//! World-space (non-repeating) 2D noise used to shape the terrain and place
//! things naturally.

use glam::Vec2;

/// Random 0..1 for an integer point.
pub fn hash2(x: i32, y: i32, seed: u32) -> f32 {
    let mut h = seed.wrapping_mul(0x9E37_79B9) ^ (x as u32).wrapping_mul(0x27D4_EB2D) ^ (y as u32).wrapping_mul(0x1656_67B1);
    h = (h ^ (h >> 15)).wrapping_mul(0x85EB_CA6B);
    h = (h ^ (h >> 13)).wrapping_mul(0xC2B2_AE35);
    h ^= h >> 16;
    (h >> 8) as f32 / 16_777_216.0
}

/// Smooth value noise, 0..1.
pub fn value(p: Vec2, seed: u32) -> f32 {
    let i = p.floor();
    let f = p - i;
    let u = f * f * (Vec2::splat(3.0) - 2.0 * f);
    let (x, y) = (i.x as i32, i.y as i32);
    let a = hash2(x, y, seed);
    let b = hash2(x + 1, y, seed);
    let c = hash2(x, y + 1, seed);
    let d = hash2(x + 1, y + 1, seed);
    let top = a + (b - a) * u.x;
    let bottom = c + (d - c) * u.x;
    top + (bottom - top) * u.y
}

/// Rotation between octaves so their grids don't line up (avoids visible
/// axis-aligned artifacts).
fn rotate(p: Vec2) -> Vec2 {
    Vec2::new(0.8 * p.x - 0.6 * p.y, 0.6 * p.x + 0.8 * p.y)
}

/// Fractal value noise, roughly 0..1.
pub fn fbm(p: Vec2, octaves: u32, seed: u32) -> f32 {
    let (mut sum, mut amp, mut norm, mut q) = (0.0, 0.5, 0.0, p);
    for o in 0..octaves {
        sum += amp * value(q, seed.wrapping_add(o * 131));
        norm += amp;
        amp *= 0.5;
        q = rotate(q) * 2.0 + Vec2::new(19.1, 7.3);
    }
    sum / norm
}

/// Ridged noise (sharp crests, like mountain ridges), 0..1.
pub fn ridged(p: Vec2, octaves: u32, seed: u32) -> f32 {
    let (mut sum, mut amp, mut norm, mut q) = (0.0, 0.5, 0.0, p);
    for o in 0..octaves {
        let n = 1.0 - (value(q, seed.wrapping_add(o * 173)) * 2.0 - 1.0).abs();
        sum += amp * n * n;
        norm += amp;
        amp *= 0.5;
        q = rotate(q) * 2.0 + Vec2::new(3.7, 11.9);
    }
    sum / norm
}

pub fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
