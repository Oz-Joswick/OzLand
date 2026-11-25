//! Writes the sample models used by the scene:
//!
//! * `assets/models/pine_tree.glb`: glTF 2.0 binary with two primitives
//!   (trunk + foliage) and two PBR materials.
//! * `assets/models/rock.obj` + `rock.mtl`: Wavefront OBJ with normals and
//!   a diffuse material.
//!
//! Run with `cargo run --example generate_sample_models`. The output is
//! already checked in under `assets/`, so this is only needed to change them.

use std::f32::consts::TAU;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use glam::{Vec2, Vec3};

/// A flat-shaded triangle soup: every triangle has its own three vertices.
#[derive(Default)]
struct Soup {
    positions: Vec<Vec3>,
    normals: Vec<Vec3>,
    /// Texture coordinates (zero for untextured geometry).
    uvs: Vec<Vec2>,
}

impl Soup {
    /// Adds a triangle wound counter-clockwise when seen from outside.
    fn tri(&mut self, a: Vec3, b: Vec3, c: Vec3) {
        let n = (b - a).cross(c - a).normalize_or_zero();
        self.positions.extend([a, b, c]);
        self.normals.extend([n, n, n]);
        self.uvs.extend([Vec2::ZERO; 3]);
    }

    /// A triangle with explicit (smooth) vertex normals, CCW from outside.
    fn tri_smooth(&mut self, p: [Vec3; 3], n: [Vec3; 3]) {
        self.positions.extend(p);
        self.normals.extend(n.map(|n| n.normalize()));
        self.uvs.extend([Vec2::ZERO; 3]);
    }

    /// A square leaf card facing `facing`, rolled by `roll`, with normals
    /// pointing away from `cluster` (and a little upward).
    fn card(&mut self, center: Vec3, facing: Vec3, roll: f32, size: f32, cluster: Vec3) {
        let a = facing.any_orthonormal_vector();
        let b = facing.cross(a);
        let (u, v) = (a * roll.cos() + b * roll.sin(), b * roll.cos() - a * roll.sin());
        let h = size * 0.5;
        let p = [center - u * h - v * h, center + u * h - v * h, center + u * h + v * h, center - u * h + v * h];
        let uv = [Vec2::new(0.0, 1.0), Vec2::new(1.0, 1.0), Vec2::new(1.0, 0.0), Vec2::new(0.0, 0.0)];
        let n = p.map(|q| (q - cluster + Vec3::Y * 0.4 * size).normalize());
        for [i, j, k] in [[0, 1, 2], [0, 2, 3]] {
            self.positions.extend([p[i], p[j], p[k]]);
            self.normals.extend([n[i], n[j], n[k]]);
            self.uvs.extend([uv[i], uv[j], uv[k]]);
        }
    }
}

impl Soup {
    /// Like `tri_smooth`, but fixes the winding so the face points the same
    /// way as its vertex normals.
    fn tri_smooth_auto(&mut self, p: [Vec3; 3], n: [Vec3; 3]) {
        let face = (p[1] - p[0]).cross(p[2] - p[0]);
        if face.dot(n[0] + n[1] + n[2]) < 0.0 {
            self.tri_smooth([p[0], p[2], p[1]], [n[0], n[2], n[1]]);
        } else {
            self.tri_smooth(p, n);
        }
    }

}

/// A tapered, smooth-shaded branch or trunk from `a` to `b`.
fn limb(soup: &mut Soup, a: Vec3, b: Vec3, r0: f32, r1: f32, sides: usize) {
    let axis = (b - a).normalize();
    let u = axis.any_orthonormal_vector();
    let v = axis.cross(u);
    let dir = |k: usize| {
        let t = k as f32 / sides as f32 * TAU;
        u * t.cos() + v * t.sin()
    };
    for i in 0..sides {
        let (d0, d1) = (dir(i), dir(i + 1));
        let (b0, b1, t0, t1) = (a + d0 * r0, a + d1 * r0, b + d0 * r1, b + d1 * r1);
        soup.tri_smooth_auto([b0, t1, t0], [d0, d1, d0]);
        soup.tri_smooth_auto([b0, b1, t1], [d0, d1, d1]);
    }
}

/// A cluster of leaves: many small alpha-cut cards scattered through an
/// ellipsoid, with "air" between them. Their normals point away from the
/// cluster centre, so the cluster shades like one soft, leafy volume
/// rather than a set of flat planes.
fn blob(soup: &mut Soup, center: Vec3, radii: Vec3, seed: f32) {
    let mut rng = Rand(0x9E37_79B9 ^ (seed * 1000.0) as u32 | 1);
    let volume = radii.x * radii.y * radii.z;
    let count = (volume * 14.0).clamp(16.0, 110.0) as usize;
    for _ in 0..count {
        let dir = loop {
            let d = Vec3::new(rng.range(-1.0, 1.0), rng.range(-1.0, 1.0), rng.range(-1.0, 1.0));
            let l = d.length();
            if l > 0.05 && l <= 1.0 {
                break d / l;
            }
        };
        // Mostly towards the outside, where leaves catch the light.
        let pos = center + dir * radii * rng.range(0.35, 1.0).sqrt();
        let jitter = Vec3::new(rng.range(-0.9, 0.9), rng.range(-0.4, 0.9), rng.range(-0.9, 0.9));
        let facing = (dir + jitter).normalize_or_zero();
        let facing = if facing == Vec3::ZERO { Vec3::Y } else { facing };
        soup.card(pos, facing, rng.range(0.0, TAU), rng.range(0.85, 1.35), center);
    }
}
/// Tiny deterministic random numbers for the generator.
struct Rand(u32);
impl Rand {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 >> 8) as f32 / 16_777_216.0
    }
    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.next()
    }
}

/// A direction tilted `tilt` radians off `up`, turned `turn` radians around it.
fn tilted(up: Vec3, tilt: f32, turn: f32) -> Vec3 {
    let u = up.any_orthonormal_vector();
    let v = up.cross(u);
    (up * tilt.cos() + (u * turn.cos() + v * turn.sin()) * tilt.sin()).normalize()
}

/// Recursive bare branches (for the dead tree).
fn branches(soup: &mut Soup, start: Vec3, dir: Vec3, length: f32, radius: f32, depth: u32, rng: &mut Rand) {
    let end = start + dir * length;
    limb(soup, start, end, radius, radius * 0.6, 6);
    if depth == 0 {
        return;
    }
    let children = 2 + (rng.next() * 2.0) as u32;
    for _ in 0..children {
        let along = rng.range(0.55, 1.0);
        let child_dir = tilted(dir, rng.range(0.4, 0.9), rng.range(0.0, TAU));
        // Bias upwards a little, like real branches reaching for light.
        let child_dir = (child_dir + Vec3::Y * 0.3).normalize();
        branches(soup, start + dir * length * along, child_dir, length * rng.range(0.45, 0.65), radius * 0.55, depth - 1, rng);
    }
}

fn angle(i: usize, sides: usize, twist: f32) -> f32 {
    (i as f32 + twist) / sides as f32 * TAU
}

/// Deterministic 0..1 noise for an integer pair.
fn jitter(i: usize, salt: u32) -> f32 {
    let mut h = (i as u32).wrapping_mul(0x9E37_79B9) ^ salt.wrapping_mul(0x85EB_CA6B);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    (h & 0xFFFF) as f32 / 65535.0
}

/// Smooth-shaded open cylinder, slightly tapered.
fn trunk(soup: &mut Soup, sides: usize, r0: f32, r1: f32, y0: f32, y1: f32) {
    let ring = |i: usize, r: f32, y: f32| {
        let a = angle(i, sides, 0.0);
        Vec3::new(r * a.cos(), y, r * a.sin())
    };
    let normal = |i: usize| {
        let a = angle(i, sides, 0.0);
        Vec3::new(a.cos(), (r0 - r1) / (y1 - y0), a.sin())
    };
    for i in 0..sides {
        let (b0, b1) = (ring(i, r0, y0), ring(i + 1, r0, y0));
        let (t0, t1) = (ring(i, r1, y1), ring(i + 1, r1, y1));
        let (n0, n1) = (normal(i), normal(i + 1));
        soup.tri_smooth([b0, t0, t1], [n0, n0, n1]);
        soup.tri_smooth([b0, t1, b1], [n0, n1, n1]);
    }
}

/// Smooth-shaded cone with a slightly ragged rim (one tier of foliage).
fn cone(soup: &mut Soup, sides: usize, radius: f32, base_y: f32, height: f32, twist: f32, salt: u32) {
    let apex = Vec3::new(0.0, base_y + height, 0.0);
    let center = Vec3::new(0.0, base_y, 0.0);
    let rim = |i: usize| {
        let a = angle(i, sides, twist);
        let r = radius * (0.88 + 0.24 * jitter(i % sides, salt));
        let droop = 0.18 * jitter(i % sides, salt + 1);
        Vec3::new(r * a.cos(), base_y - droop, r * a.sin())
    };
    // Normal of a cone surface at angle a: outward and tilted up by the slope.
    let normal = |a: f32| Vec3::new(a.cos() * height, radius, a.sin() * height);
    for i in 0..sides {
        let (a0, a1) = (angle(i, sides, twist), angle(i + 1, sides, twist));
        let mid = (a0 + a1) * 0.5;
        soup.tri_smooth([rim(i + 1), rim(i), apex], [normal(a1), normal(a0), normal(mid)]);
        soup.tri(center, rim(i), rim(i + 1)); // underside, flat
    }
}

fn srgb_to_linear(c: u8) -> f32 {
    let c = c as f32 / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn linear_rgba(r: u8, g: u8, b: u8) -> [f32; 4] {
    [srgb_to_linear(r), srgb_to_linear(g), srgb_to_linear(b), 1.0]
}

/// A cluster of pointed leaves on a twig, with clear "air" between them,
/// as a 512x512 PNG. The leaves are near-white so each material's colour
/// tints them (green, orange, pink...). Transparent pixels carry the leaf
/// colour too, so mipmaps don't get dark fringes.
fn leaf_texture_png() -> Vec<u8> {
    const S: u32 = 512;
    let mut img = image::RgbaImage::from_pixel(S, S, image::Rgba([210, 220, 200, 0]));
    let mut rng = Rand(0xBEEF_1234);
    let center = Vec2::splat(S as f32 * 0.5);
    // Twig from the bottom of the card to its middle.
    let twig_end = center + Vec2::new(0.0, -20.0);
    for y in 0..S {
        for x in 0..S {
            let p = Vec2::new(x as f32, y as f32);
            let d = {
                let (a, b) = (Vec2::new(S as f32 * 0.5, S as f32), twig_end);
                let t = ((p - a).dot(b - a) / (b - a).length_squared()).clamp(0.0, 1.0);
                (p - (a + (b - a) * t)).length()
            };
            if d < 4.0 {
                img.put_pixel(x, y, image::Rgba([120, 105, 90, 255]));
            }
        }
    }
    // Leaves radiating from the twig, each a pointed ellipse with a midrib.
    for i in 0..15 {
        let a = i as f32 / 15.0 * std::f32::consts::TAU + rng.range(-0.2, 0.2);
        let dir = Vec2::new(a.cos(), a.sin());
        let length = rng.range(95.0, 140.0);
        let width = length * rng.range(0.3, 0.4);
        let base = twig_end + dir * rng.range(15.0, 70.0);
        let shade = rng.range(0.78, 1.0);
        for y in 0..S {
            for x in 0..S {
                let q = Vec2::new(x as f32, y as f32) - base;
                let along = q.dot(dir);
                let across = q.dot(Vec2::new(-dir.y, dir.x));
                if !(0.0..length).contains(&along) {
                    continue;
                }
                // Widest a third of the way along, pointed at the tip.
                let t = along / length;
                let half = width * 0.5 * (t * std::f32::consts::PI).sin().powf(0.8) * (1.0 - t * 0.3);
                if across.abs() > half {
                    continue;
                }
                let rib = if across.abs() < 2.0 { 0.82 } else { 1.0 };
                let edge = 0.9 + 0.1 * (1.0 - across.abs() / half.max(1.0));
                let v = (255.0 * shade * rib * edge) as u8;
                img.put_pixel(x, y, image::Rgba([v, v, (v as f32 * 0.93) as u8, 255]));
            }
        }
    }
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .expect("encode leaf texture");
    png
}

// ---------------------------------------------------------------------------
// GLB writer
// ---------------------------------------------------------------------------

struct Primitive<'a> {
    soup: &'a Soup,
    material_name: &'a str,
    base_color: [f32; 4],
}

fn push_f32s(bin: &mut Vec<u8>, values: impl IntoIterator<Item = f32>) {
    for v in values {
        bin.extend_from_slice(&v.to_le_bytes());
    }
}

fn write_glb(path: &Path, mesh_name: &str, prims: &[Primitive]) -> std::io::Result<()> {
    let mut bin = Vec::new();
    let mut views = Vec::new();
    let mut accessors = Vec::new();
    let mut gltf_prims = Vec::new();
    let mut materials = Vec::new();

    for (mi, prim) in prims.iter().enumerate() {
        let count = prim.soup.positions.len();
        let (min, max) = prim.soup.positions.iter().fold(
            (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
            |(lo, hi), &p| (lo.min(p), hi.max(p)),
        );

        // POSITION
        let offset = bin.len();
        push_f32s(&mut bin, prim.soup.positions.iter().flat_map(|p| p.to_array()));
        views.push(format!(
            r#"{{"buffer":0,"byteOffset":{offset},"byteLength":{},"target":34962}}"#,
            bin.len() - offset
        ));
        accessors.push(format!(
            r#"{{"bufferView":{},"componentType":5126,"count":{count},"type":"VEC3","min":[{:?},{:?},{:?}],"max":[{:?},{:?},{:?}]}}"#,
            views.len() - 1, min.x, min.y, min.z, max.x, max.y, max.z
        ));
        let position_accessor = accessors.len() - 1;

        // NORMAL
        let offset = bin.len();
        push_f32s(&mut bin, prim.soup.normals.iter().flat_map(|n| n.to_array()));
        views.push(format!(
            r#"{{"buffer":0,"byteOffset":{offset},"byteLength":{},"target":34962}}"#,
            bin.len() - offset
        ));
        accessors.push(format!(
            r#"{{"bufferView":{},"componentType":5126,"count":{count},"type":"VEC3"}}"#,
            views.len() - 1
        ));
        let normal_accessor = accessors.len() - 1;

        // indices (u32)
        let offset = bin.len();
        for i in 0..count as u32 {
            bin.extend_from_slice(&i.to_le_bytes());
        }
        views.push(format!(
            r#"{{"buffer":0,"byteOffset":{offset},"byteLength":{},"target":34963}}"#,
            bin.len() - offset
        ));
        accessors.push(format!(
            r#"{{"bufferView":{},"componentType":5125,"count":{count},"type":"SCALAR"}}"#,
            views.len() - 1
        ));
        let index_accessor = accessors.len() - 1;

        // TEXCOORD_0
        let offset = bin.len();
        push_f32s(&mut bin, prim.soup.uvs.iter().flat_map(|t| t.to_array()));
        views.push(format!(
            r#"{{"buffer":0,"byteOffset":{offset},"byteLength":{},"target":34962}}"#,
            bin.len() - offset
        ));
        accessors.push(format!(
            r#"{{"bufferView":{},"componentType":5126,"count":{count},"type":"VEC2"}}"#,
            views.len() - 1
        ));
        let uv_accessor = accessors.len() - 1;

        gltf_prims.push(format!(
            r#"{{"attributes":{{"POSITION":{position_accessor},"NORMAL":{normal_accessor},"TEXCOORD_0":{uv_accessor}}},"indices":{index_accessor},"material":{mi}}}"#
        ));
        let [r, g, b, a] = prim.base_color;
        // Leaf cards (anything with texture coordinates) use the leaf
        // texture, cut out by its alpha and visible from both sides.
        let leaves = prim.soup.uvs.iter().any(|t| *t != Vec2::ZERO);
        let (texture, extra) = if leaves {
            (r#","baseColorTexture":{"index":0}"#, r#","alphaMode":"MASK","alphaCutoff":0.4,"doubleSided":true"#)
        } else {
            ("", "")
        };
        materials.push(format!(
            r#"{{"name":"{}","pbrMetallicRoughness":{{"baseColorFactor":[{r:?},{g:?},{b:?},{a:?}]{texture},"metallicFactor":0.0,"roughnessFactor":0.85}}{extra}}}"#,
            prim.material_name
        ));
    }

    // Embed the leaf texture once, if any primitive uses it.
    let mut images = String::new();
    if prims.iter().any(|p| p.soup.uvs.iter().any(|t| *t != Vec2::ZERO)) {
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        let png = leaf_texture_png();
        let offset = bin.len();
        bin.extend_from_slice(&png);
        views.push(format!(r#"{{"buffer":0,"byteOffset":{offset},"byteLength":{}}}"#, png.len()));
        images = format!(
            r#","images":[{{"bufferView":{},"mimeType":"image/png"}}],"textures":[{{"source":0,"sampler":0}}],"samplers":[{{"magFilter":9729,"minFilter":9987,"wrapS":33071,"wrapT":33071}}]"#,
            views.len() - 1
        );
    }

    let mut json = String::new();
    write!(
        json,
        r#"{{"asset":{{"version":"2.0","generator":"generate_sample_models"}},"scene":0,"scenes":[{{"nodes":[0]}}],"nodes":[{{"name":"{mesh_name}","mesh":0}}],"meshes":[{{"name":"{mesh_name}","primitives":[{}]}}],"materials":[{}],"buffers":[{{"byteLength":{}}}],"bufferViews":[{}],"accessors":[{}]{images}}}"#,
        gltf_prims.join(","),
        materials.join(","),
        bin.len(),
        views.join(","),
        accessors.join(","),
    )
    .unwrap();

    // GLB container: 12-byte header, then JSON and BIN chunks padded to 4 bytes.
    let mut json = json.into_bytes();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let total = 12 + 8 + json.len() + 8 + bin.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin);
    fs::write(path, out)
}

// ---------------------------------------------------------------------------
// Models
// ---------------------------------------------------------------------------

fn pine_tree(dir: &Path) -> std::io::Result<()> {
    let mut bark = Soup::default();
    trunk(&mut bark, 10, 0.26, 0.16, 0.0, 2.6);

    let mut foliage = Soup::default();
    cone(&mut foliage, 14, 1.6, 1.9, 2.0, 0.0, 1);
    cone(&mut foliage, 14, 1.3, 2.7, 1.8, 0.5, 2);
    cone(&mut foliage, 14, 1.0, 3.5, 1.6, 0.25, 3);
    cone(&mut foliage, 14, 0.65, 4.2, 1.3, 0.75, 4);

    write_glb(
        &dir.join("pine_tree.glb"),
        "PineTree",
        &[
            Primitive { soup: &bark, material_name: "Bark", base_color: linear_rgba(96, 66, 44) },
            Primitive { soup: &foliage, material_name: "Needles", base_color: linear_rgba(40, 88, 44) },
        ],
    )
}

/// Five more tree species, each with a clearly different silhouette.
fn more_trees(dir: &Path) -> std::io::Result<()> {
    let dir = dir.join("trees");
    fs::create_dir_all(&dir)?;
    let mut rng = Rand(0x1234_5678);

    // Oak: thick trunk forking into limbs, under a broad clumpy crown.
    let (mut bark, mut leaves) = (Soup::default(), Soup::default());
    limb(&mut bark, Vec3::ZERO, Vec3::new(0.0, 3.0, 0.0), 0.45, 0.32, 10);
    let mut crown = vec![(Vec3::new(0.0, 6.2, 0.0), Vec3::new(2.3, 1.7, 2.3))];
    for i in 0..4 {
        let a = i as f32 / 4.0 * TAU + 0.4;
        let tip = Vec3::new(a.cos() * 2.0, 4.8 + rng.range(-0.3, 0.4), a.sin() * 2.0);
        limb(&mut bark, Vec3::new(0.0, 2.7, 0.0), tip, 0.26, 0.14, 7);
        crown.push((tip + Vec3::Y * 0.4, Vec3::new(1.8, 1.4, 1.8)));
    }
    for (i, (c, r)) in crown.iter().enumerate() {
        blob(&mut leaves, *c, *r, i as f32);
    }
    write_glb(
        &dir.join("oak.glb"),
        "Oak",
        &[
            Primitive { soup: &bark, material_name: "OakBark", base_color: linear_rgba(88, 70, 54) },
            Primitive { soup: &leaves, material_name: "OakLeaves", base_color: linear_rgba(62, 104, 38) },
        ],
    )?;

    // Autumn maple: a rounded crown in orange and red.
    let (mut bark, mut orange, mut red) = (Soup::default(), Soup::default(), Soup::default());
    limb(&mut bark, Vec3::ZERO, Vec3::new(0.15, 3.4, 0.0), 0.3, 0.2, 9);
    for i in 0..3 {
        let a = i as f32 / 3.0 * TAU;
        limb(&mut bark, Vec3::new(0.1, 3.0, 0.0), Vec3::new(a.cos() * 1.3, 4.6, a.sin() * 1.3), 0.17, 0.09, 6);
    }
    blob(&mut orange, Vec3::new(0.0, 5.4, 0.0), Vec3::new(2.4, 2.1, 2.4), 1.0);
    for i in 0..5 {
        let a = i as f32 / 5.0 * TAU;
        let c = Vec3::new(a.cos() * 1.7, 5.0 + rng.range(-0.4, 0.8), a.sin() * 1.7);
        blob(if i % 2 == 0 { &mut red } else { &mut orange }, c, Vec3::splat(1.2), 2.0 + i as f32);
    }
    write_glb(
        &dir.join("maple.glb"),
        "Maple",
        &[
            Primitive { soup: &bark, material_name: "MapleBark", base_color: linear_rgba(72, 56, 46) },
            Primitive { soup: &orange, material_name: "MapleOrange", base_color: linear_rgba(214, 112, 34) },
            Primitive { soup: &red, material_name: "MapleRed", base_color: linear_rgba(176, 48, 30) },
        ],
    )?;

    // Birch: a slender, slightly leaning white trunk with small airy clusters.
    let (mut bark, mut leaves) = (Soup::default(), Soup::default());
    let top = Vec3::new(0.35, 8.0, 0.1);
    limb(&mut bark, Vec3::ZERO, top, 0.16, 0.07, 8);
    for i in 0..7 {
        let h = 4.2 + i as f32 * 0.55;
        let a = i as f32 * 2.3;
        let on_trunk = top * (h / top.y);
        let c = on_trunk + Vec3::new(a.cos() * 0.7, 0.0, a.sin() * 0.7);
        limb(&mut bark, on_trunk, c, 0.05, 0.03, 5);
        blob(&mut leaves, c + Vec3::Y * 0.2, Vec3::new(0.8, 1.15, 0.8), 10.0 + i as f32);
    }
    write_glb(
        &dir.join("birch.glb"),
        "Birch",
        &[
            Primitive { soup: &bark, material_name: "BirchBark", base_color: linear_rgba(222, 218, 206) },
            Primitive { soup: &leaves, material_name: "BirchLeaves", base_color: linear_rgba(132, 168, 62) },
        ],
    )?;

    // Spruce: very tall and narrow, many tight dark tiers.
    let (mut bark, mut needles) = (Soup::default(), Soup::default());
    trunk(&mut bark, 10, 0.3, 0.1, 0.0, 10.5);
    for i in 0..9 {
        let t = i as f32 / 8.0;
        let radius = 2.1 * (1.0 - t) + 0.35;
        cone(&mut needles, 12, radius, 1.4 + t * 9.0, 1.6 + (1.0 - t) * 0.6, i as f32 * 0.37, 20 + i);
    }
    write_glb(
        &dir.join("spruce.glb"),
        "Spruce",
        &[
            Primitive { soup: &bark, material_name: "SpruceBark", base_color: linear_rgba(70, 52, 40) },
            Primitive { soup: &needles, material_name: "SpruceNeedles", base_color: linear_rgba(26, 62, 50) },
        ],
    )?;

    // Weeping willow: a short trunk, arching limbs, curtains of drooping leaves.
    let (mut bark, mut leaves) = (Soup::default(), Soup::default());
    limb(&mut bark, Vec3::ZERO, Vec3::new(0.0, 2.6, 0.0), 0.42, 0.3, 10);
    blob(&mut leaves, Vec3::new(0.0, 5.0, 0.0), Vec3::new(2.4, 1.3, 2.4), 30.0);
    for i in 0..10 {
        let a = i as f32 / 10.0 * TAU + rng.range(-0.2, 0.2);
        let tip = Vec3::new(a.cos() * 2.6, 4.6, a.sin() * 2.6);
        limb(&mut bark, Vec3::new(0.0, 2.4, 0.0), tip, 0.16, 0.06, 6);
        // Long, thin hanging strands reaching most of the way to the ground.
        let hang = tip + Vec3::new(a.cos() * 0.4, -1.6, a.sin() * 0.4);
        blob(&mut leaves, hang, Vec3::new(0.55, rng.range(1.7, 2.3), 0.55), 31.0 + i as f32);
    }
    write_glb(
        &dir.join("willow.glb"),
        "Willow",
        &[
            Primitive { soup: &bark, material_name: "WillowBark", base_color: linear_rgba(84, 72, 58) },
            Primitive { soup: &leaves, material_name: "WillowLeaves", base_color: linear_rgba(120, 150, 70) },
        ],
    )?;

    // Cherry blossom: a spreading crown of pink and pale-pink clouds.
    let (mut bark, mut pink, mut pale) = (Soup::default(), Soup::default(), Soup::default());
    limb(&mut bark, Vec3::ZERO, Vec3::new(-0.2, 2.2, 0.1), 0.26, 0.18, 9);
    for i in 0..5 {
        let a = i as f32 / 5.0 * TAU + 0.3;
        let tip = Vec3::new(a.cos() * 2.2, 3.6 + rng.range(-0.3, 0.5), a.sin() * 2.2);
        limb(&mut bark, Vec3::new(-0.2, 2.0, 0.1), tip, 0.13, 0.06, 6);
        blob(if i % 2 == 0 { &mut pink } else { &mut pale }, tip + Vec3::Y * 0.4, Vec3::new(1.5, 1.0, 1.5), 40.0 + i as f32);
    }
    blob(&mut pink, Vec3::new(0.0, 4.6, 0.0), Vec3::new(1.8, 1.1, 1.8), 46.0);
    write_glb(
        &dir.join("cherry.glb"),
        "Cherry",
        &[
            Primitive { soup: &bark, material_name: "CherryBark", base_color: linear_rgba(70, 46, 42) },
            Primitive { soup: &pink, material_name: "Blossom", base_color: linear_rgba(238, 150, 186) },
            Primitive { soup: &pale, material_name: "BlossomPale", base_color: linear_rgba(250, 210, 225) },
        ],
    )?;

    // Poplar: a tall, narrow green column.
    let (mut bark, mut leaves) = (Soup::default(), Soup::default());
    limb(&mut bark, Vec3::ZERO, Vec3::new(0.0, 3.0, 0.0), 0.28, 0.2, 8);
    for i in 0..4 {
        let y = 4.5 + i as f32 * 2.0;
        let r = 1.25 - i as f32 * 0.18;
        blob(&mut leaves, Vec3::new(rng.range(-0.15, 0.15), y, rng.range(-0.15, 0.15)), Vec3::new(r, 1.8, r), 50.0 + i as f32);
    }
    write_glb(
        &dir.join("poplar.glb"),
        "Poplar",
        &[
            Primitive { soup: &bark, material_name: "PoplarBark", base_color: linear_rgba(96, 86, 70) },
            Primitive { soup: &leaves, material_name: "PoplarLeaves", base_color: linear_rgba(76, 122, 44) },
        ],
    )?;

    // Dead tree: a grey trunk with bare, forking branches.
    let mut wood = Soup::default();
    limb(&mut wood, Vec3::ZERO, Vec3::new(0.0, 3.2, 0.0), 0.34, 0.2, 9);
    for i in 0..4 {
        let dir = tilted(Vec3::Y, rng.range(0.5, 0.9), i as f32 / 4.0 * TAU + rng.range(0.0, 0.8));
        let start = Vec3::new(0.0, 2.2 + i as f32 * 0.35, 0.0);
        branches(&mut wood, start, dir, rng.range(1.6, 2.4), 0.16, 2, &mut rng);
    }
    branches(&mut wood, Vec3::new(0.0, 3.2, 0.0), Vec3::Y, 1.8, 0.18, 2, &mut rng);
    write_glb(
        &dir.join("dead.glb"),
        "DeadTree",
        &[Primitive { soup: &wood, material_name: "DeadWood", base_color: linear_rgba(112, 104, 96) }],
    )
}

/// A simple placeholder player body, 1.8 m tall, feet at the origin, facing
/// +X. Primitive 0 is everything but the head; primitive 1 is the head (so
/// it can be hidden in first person).
fn player(dir: &Path) -> std::io::Result<()> {
    let (mut body, mut head) = (Soup::default(), Soup::default());
    for side in [-1.0f32, 1.0] {
        let z = side * 0.11;
        // Leg: hip to ankle, plus a foot pointing forward.
        limb(&mut body, Vec3::new(0.0, 0.92, z), Vec3::new(0.0, 0.08, z), 0.085, 0.06, 8);
        limb(&mut body, Vec3::new(-0.04, 0.05, z), Vec3::new(0.2, 0.05, z), 0.055, 0.045, 6);
        // Arm: shoulder to hand, hanging slightly out from the body.
        limb(&mut body, Vec3::new(0.0, 1.42, side * 0.23), Vec3::new(0.03, 0.82, side * 0.27), 0.06, 0.045, 8);
    }
    // Torso and neck.
    limb(&mut body, Vec3::new(0.0, 0.88, 0.0), Vec3::new(0.0, 1.45, 0.0), 0.16, 0.19, 12);
    limb(&mut body, Vec3::new(0.0, 1.45, 0.0), Vec3::new(0.0, 1.55, 0.0), 0.06, 0.055, 8);
    // Head, with a small nose so the facing direction is visible.
    limb(&mut head, Vec3::new(0.0, 1.55, 0.0), Vec3::new(0.0, 1.8, 0.0), 0.11, 0.1, 12);
    limb(&mut head, Vec3::new(0.09, 1.66, 0.0), Vec3::new(0.14, 1.65, 0.0), 0.02, 0.012, 6);
    write_glb(
        &dir.join("player.glb"),
        "Player",
        &[
            Primitive { soup: &body, material_name: "Clothes", base_color: linear_rgba(70, 90, 130) },
            Primitive { soup: &head, material_name: "Skin", base_color: linear_rgba(220, 180, 150) },
        ],
    )
}

fn rock(dir: &Path) -> std::io::Result<()> {
    // Icosahedron, subdivided once, with deterministic per-vertex bumps.
    let t = (1.0 + 5f32.sqrt()) / 2.0;
    let base: Vec<Vec3> = [
        (-1.0, t, 0.0), (1.0, t, 0.0), (-1.0, -t, 0.0), (1.0, -t, 0.0),
        (0.0, -1.0, t), (0.0, 1.0, t), (0.0, -1.0, -t), (0.0, 1.0, -t),
        (t, 0.0, -1.0), (t, 0.0, 1.0), (-t, 0.0, -1.0), (-t, 0.0, 1.0),
    ]
    .iter()
    .map(|&(x, y, z)| Vec3::new(x, y, z).normalize())
    .collect();
    let faces = [
        [0, 11, 5], [0, 5, 1], [0, 1, 7], [0, 7, 10], [0, 10, 11],
        [1, 5, 9], [5, 11, 4], [11, 10, 2], [10, 7, 6], [7, 1, 8],
        [3, 9, 4], [3, 4, 2], [3, 2, 6], [3, 6, 8], [3, 8, 9],
        [4, 9, 5], [2, 4, 11], [6, 2, 10], [8, 6, 7], [9, 8, 1],
    ];

    let bump = |p: Vec3| {
        let h = (p.x * 12.9898 + p.y * 78.233 + p.z * 37.719).sin() * 43758.547;
        let n = h - h.floor(); // 0..1, same for the same point
        let p = p * (0.82 + 0.3 * n);
        Vec3::new(p.x * 1.2, p.y * 0.7, p.z) // wider than tall
    };

    let mut soup = Soup::default();
    for [a, b, c] in faces {
        let (a, b, c) = (base[a], base[b], base[c]);
        let (ab, bc, ca) = ((a + b).normalize(), (b + c).normalize(), (c + a).normalize());
        for [p, q, r] in [[a, ab, ca], [ab, b, bc], [ca, bc, c], [ab, bc, ca]] {
            let (p, q, r) = (bump(p), bump(q), bump(r));
            // Make sure each face winds outward.
            let n = (q - p).cross(r - p);
            if n.dot(p + q + r) < 0.0 {
                soup.tri(p, r, q);
            } else {
                soup.tri(p, q, r);
            }
        }
    }

    let mut obj = String::from("# Low-poly rock\nmtllib rock.mtl\no Rock\n");
    for p in &soup.positions {
        writeln!(obj, "v {:.5} {:.5} {:.5}", p.x, p.y, p.z).unwrap();
    }
    for n in &soup.normals {
        writeln!(obj, "vn {:.5} {:.5} {:.5}", n.x, n.y, n.z).unwrap();
    }
    obj.push_str("usemtl Stone\n");
    for i in 0..soup.positions.len() / 3 {
        let (a, b, c) = (3 * i + 1, 3 * i + 2, 3 * i + 3);
        writeln!(obj, "f {a}//{a} {b}//{b} {c}//{c}").unwrap();
    }
    fs::write(dir.join("rock.obj"), obj)?;

    let [r, g, b, _] = linear_rgba(128, 124, 118);
    fs::write(
        dir.join("rock.mtl"),
        format!("newmtl Stone\nKd {r:.4} {g:.4} {b:.4}\n"),
    )
}

fn main() -> std::io::Result<()> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets").join("models");
    fs::create_dir_all(&dir)?;
    pine_tree(&dir)?;
    more_trees(&dir)?;
    player(&dir)?;
    rock(&dir)?;
    println!("wrote sample models to {}", dir.display());
    Ok(())
}
