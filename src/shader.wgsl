// Forward renderer: sun (directional light, two shadow cascades) +
// hemispheric ambient with sky occlusion + Blinn-Phong specular, normal and
// parallax mapping, splat-mapped terrain, GPU grass and flowers, water,
// procedural sky, fog and ACES tone mapping. Lighting is in linear HDR.
//
// Depth is reverse-Z (1 = near, 0 = infinitely far) in the main pass; the
// depth-only shadow passes use the ordinary 0..1 convention.

struct Globals {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    // Matrix used by depth-only passes. In the main globals this is the
    // near (sharp) shadow cascade around the player.
    light_view_proj: mat4x4<f32>,
    // The far shadow cascade, covering the whole world.
    far_light_view_proj: mat4x4<f32>,
    // Top-down orthographic view used to find what is sheltered from the sky.
    sky_view_proj: mat4x4<f32>,
    // xyz = camera position, w = time in seconds.
    camera_pos: vec4<f32>,
    // Direction the sunlight travels (from the sun towards the ground).
    sun_dir: vec4<f32>,
    sun_color: vec4<f32>,
    sky_zenith: vec4<f32>,
    sky_horizon: vec4<f32>,
    sky_ambient: vec4<f32>,
    ground_ambient: vec4<f32>,
    // x = fog start, y = fog end, z = exposure, w = shadow-map texel size (uv).
    params: vec4<f32>,
    // x = parallax scale (0 = off), y = water level, z = terrain half size,
    // w = terrain grid cell size.
    extra: vec4<f32>,
};

struct Material {
    base_color: vec4<f32>,
    // x = alpha cutoff, y = specular strength, z = shininess, w = parallax depth.
    shading: vec4<f32>,
    // x = detail noise amount, y = detail noise frequency (per meter),
    // z = macro color variation, w = ground occlusion strength.
    detail: vec4<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;
// Terrain heights (R32Float), one texel per grid point.
@group(0) @binding(1) var height_map: texture_2d<f32>;

@group(1) @binding(0) var<uniform> material: Material;
@group(1) @binding(1) var base_texture: texture_2d<f32>;
@group(1) @binding(2) var normal_texture: texture_2d<f32>;
@group(1) @binding(3) var material_sampler: sampler;

@group(2) @binding(0) var shadow_map: texture_depth_2d_array;
@group(2) @binding(1) var shadow_sampler: sampler_comparison;
@group(2) @binding(2) var sky_occlusion_map: texture_depth_2d;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>,
    @location(4) tangent: vec4<f32>,
};

// Per-instance model matrix, its normal matrix (inverse-transpose of the
// upper 3x3, computed on the CPU) and a color tint.
struct InstanceIn {
    @location(5) model_0: vec4<f32>,
    @location(6) model_1: vec4<f32>,
    @location(7) model_2: vec4<f32>,
    @location(8) model_3: vec4<f32>,
    @location(9) normal_0: vec3<f32>,
    @location(10) normal_1: vec3<f32>,
    @location(11) normal_2: vec3<f32>,
    @location(12) tint: vec4<f32>,
};

struct VertexOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>,
    @location(4) tangent: vec4<f32>,
};

fn instance_model(i: InstanceIn) -> mat4x4<f32> {
    return mat4x4<f32>(i.model_0, i.model_1, i.model_2, i.model_3);
}

@vertex
fn vs_main(vertex: VertexIn, instance: InstanceIn) -> VertexOut {
    let model = instance_model(instance);
    let normal_matrix = mat3x3<f32>(instance.normal_0, instance.normal_1, instance.normal_2);
    let world = model * vec4<f32>(vertex.position, 1.0);

    var out: VertexOut;
    out.clip_position = globals.view_proj * world;
    out.world_position = world.xyz;
    out.normal = normal_matrix * vertex.normal;
    let model3 = mat3x3<f32>(model[0].xyz, model[1].xyz, model[2].xyz);
    out.tangent = vec4<f32>(model3 * vertex.tangent.xyz, vertex.tangent.w);
    out.uv = vertex.uv;
    out.color = vertex.color * instance.tint;
    return out;
}

// Depth-only pass (shadow cascades, sky occlusion).
struct ShadowOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_shadow(vertex: VertexIn, instance: InstanceIn) -> ShadowOut {
    var out: ShadowOut;
    out.clip_position = globals.light_view_proj * instance_model(instance) * vec4<f32>(vertex.position, 1.0);
    out.uv = vertex.uv;
    return out;
}

// Cut-out materials (leaf cards) only cast shadows where they're solid.
@fragment
fn fs_shadow(in: ShadowOut) {
    if material.shading.x > 0.0 {
        let alpha = textureSampleLevel(base_texture, material_sampler, in.uv, 0.0).a * material.base_color.a;
        if alpha < material.shading.x {
            discard;
        }
    }
}

// ---------------------------------------------------------------------------
// Terrain height
// ---------------------------------------------------------------------------

// Height of the terrain surface at a world XZ position. Uses the same
// triangle split as the terrain mesh and the CPU (`Terrain::height_at`).
fn terrain_height(xz: vec2<f32>) -> f32 {
    let half_size = globals.extra.z;
    let cell = globals.extra.w;
    let max_index = vec2<i32>(textureDimensions(height_map)) - 1;
    let g = clamp((xz + half_size) / cell, vec2<f32>(0.0), vec2<f32>(max_index) - 0.0001);
    let i = vec2<i32>(floor(g));
    let f = g - vec2<f32>(i);
    let h00 = textureLoad(height_map, i, 0).r;
    let h10 = textureLoad(height_map, min(i + vec2<i32>(1, 0), max_index), 0).r;
    let h01 = textureLoad(height_map, min(i + vec2<i32>(0, 1), max_index), 0).r;
    let h11 = textureLoad(height_map, min(i + vec2<i32>(1, 1), max_index), 0).r;
    if f.x >= f.y {
        return h00 + (h10 - h00) * f.x + (h11 - h10) * f.y;
    }
    return h00 + (h11 - h01) * f.x + (h01 - h00) * f.y;
}

// ---------------------------------------------------------------------------
// Noise
// ---------------------------------------------------------------------------

fn pcg(v: u32) -> u32 {
    let state = v * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

// Random 0..1 for an integer lattice point (exact integer hashing, so it
// behaves the same for negative coordinates).
fn hash3(p: vec3<f32>) -> f32 {
    let c = bitcast<vec3<u32>>(vec3<i32>(p));
    return f32(pcg(c.x ^ pcg(c.y ^ pcg(c.z)))) / 4294967295.0;
}

fn value_noise(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = mix(hash3(i), hash3(i + vec3<f32>(1.0, 0.0, 0.0)), u.x);
    let b = mix(hash3(i + vec3<f32>(0.0, 1.0, 0.0)), hash3(i + vec3<f32>(1.0, 1.0, 0.0)), u.x);
    let c = mix(hash3(i + vec3<f32>(0.0, 0.0, 1.0)), hash3(i + vec3<f32>(1.0, 0.0, 1.0)), u.x);
    let d = mix(hash3(i + vec3<f32>(0.0, 1.0, 1.0)), hash3(i + vec3<f32>(1.0, 1.0, 1.0)), u.x);
    return mix(mix(a, b, u.y), mix(c, d, u.y), u.z);
}

fn fbm(p: vec3<f32>) -> f32 {
    var sum = 0.0;
    var amp = 0.5;
    var q = p;
    for (var i = 0; i < 4; i++) {
        sum += amp * value_noise(q);
        q = q * 2.03 + vec3<f32>(17.1, 3.7, 9.2);
        amp *= 0.5;
    }
    return sum / 0.9375;
}

// ---------------------------------------------------------------------------
// Sky
// ---------------------------------------------------------------------------

fn to_sun() -> vec3<f32> {
    return normalize(-globals.sun_dir.xyz);
}

// Sky radiance in direction `dir`, without the sun disc and clouds (also
// used as the fog color so distant objects fade into the horizon).
fn sky_base(dir: vec3<f32>) -> vec3<f32> {
    let up = clamp(dir.y, 0.0, 1.0);
    var color = mix(globals.sky_horizon.rgb, globals.sky_zenith.rgb, pow(up, 0.6));
    // Below the horizon fade into a hazy ground color.
    let down = clamp(-dir.y * 4.0, 0.0, 1.0);
    color = mix(color, globals.sky_horizon.rgb * 0.75, down);
    let s = max(dot(dir, to_sun()), 0.0);
    color += globals.sun_color.rgb * (0.025 * pow(s, 8.0) + 0.12 * pow(s, 64.0));
    return color;
}

fn sky(dir: vec3<f32>) -> vec3<f32> {
    var color = sky_base(dir);
    let s = max(dot(dir, to_sun()), 0.0);
    color += globals.sun_color.rgb * 12.0 * smoothstep(0.9993, 0.9997, s);

    if dir.y > 0.0 {
        // Clouds on a flat layer, drifting slowly.
        let p = dir.xz / (dir.y + 0.12) * 1.3 + vec2<f32>(globals.camera_pos.w * 0.006, 0.0);
        let n = fbm(vec3<f32>(p, globals.camera_pos.w * 0.01));
        let cover = smoothstep(0.5, 0.78, n) * smoothstep(0.0, 0.2, dir.y);
        let lit = 0.75 + 0.5 * pow(s, 4.0);
        let cloud = mix(globals.sky_horizon.rgb * 1.05, vec3<f32>(1.0, 0.98, 0.95) * 1.6, lit * (1.0 - n * 0.4));
        color = mix(color, cloud, cover * 0.85);
    }
    return color;
}

// ACES filmic tone curve (Narkowicz fit).
fn tonemap(x: vec3<f32>) -> vec3<f32> {
    let v = x * globals.params.z;
    return clamp((v * (2.51 * v + 0.03)) / (v * (2.43 * v + 0.59) + 0.14), vec3<f32>(0.0), vec3<f32>(1.0));
}

// Distance haze in the color of the sky behind the point. Exponential, so
// the middle distance stays clear while far mountains soften into the sky.
fn apply_fog(color: vec3<f32>, world_pos: vec3<f32>) -> vec3<f32> {
    let to_point = world_pos - globals.camera_pos.xyz;
    let beyond = max(length(to_point) - globals.params.x, 0.0) / (globals.params.y - globals.params.x);
    let fog = (1.0 - exp(-beyond * 1.6)) * 0.9;
    return mix(color, sky_base(normalize(to_point)), fog);
}

struct SkyOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

// One triangle covering the screen.
@vertex
fn vs_sky(@builtin(vertex_index) index: u32) -> SkyOut {
    let xy = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u)) * 2.0 - 1.0;
    var out: SkyOut;
    out.clip_position = vec4<f32>(xy, 0.0, 1.0);
    out.ndc = xy;
    return out;
}

@fragment
fn fs_sky(in: SkyOut) -> @location(0) vec4<f32> {
    // Any depth works for the direction; with reverse-Z, 1 is the near plane.
    let p = globals.inv_view_proj * vec4<f32>(in.ndc, 1.0, 1.0);
    let dir = normalize(p.xyz / p.w - globals.camera_pos.xyz);
    return vec4<f32>(tonemap(sky(dir)), 1.0);
}

// ---------------------------------------------------------------------------
// Lighting
// ---------------------------------------------------------------------------

// Light-space coordinates in a cascade: xy = shadow-map UV, z = depth.
fn cascade_coord(m: mat4x4<f32>, pos: vec3<f32>) -> vec3<f32> {
    let p = m * vec4<f32>(pos, 1.0);
    let ndc = p.xyz / p.w;
    return vec3<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5, ndc.z);
}

// Soft (3x3, hardware-filtered) shadow lookup in one cascade.
fn cascade_shadow(layer: i32, c: vec3<f32>, bias: f32) -> f32 {
    let texel = globals.params.w;
    var sum = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let offset = vec2<f32>(f32(x), f32(y)) * texel * 1.5;
            sum += textureSampleCompareLevel(shadow_map, shadow_sampler, c.xy + offset, layer, c.z - bias);
        }
    }
    return sum / 9.0;
}

fn in_cascade(c: vec3<f32>) -> bool {
    return all(c.xy > vec2<f32>(0.0)) && all(c.xy < vec2<f32>(1.0)) && c.z > 0.0 && c.z < 1.0;
}

// 0 = fully shadowed, 1 = fully lit. Uses the sharp cascade near the
// player, blending into the coarse whole-world cascade towards its edge.
fn sun_visibility(world_pos: vec3<f32>, normal: vec3<f32>) -> f32 {
    // Push lookups off the surface to avoid self-shadowing ("acne").
    let far_coord = cascade_coord(globals.far_light_view_proj, world_pos + normal * 0.3);
    let near_coord = cascade_coord(globals.light_view_proj, world_pos + normal * 0.06);
    // Depth ranges: ~400 m near (bias 4 cm), ~900 m far (bias 18 cm).
    if in_cascade(near_coord) {
        let near = cascade_shadow(0, near_coord, 0.0001);
        let edge = max(abs(near_coord.x - 0.5), abs(near_coord.y - 0.5)) * 2.0;
        let blend = smoothstep(0.8, 1.0, edge);
        if blend <= 0.0 {
            return near;
        }
        return mix(near, cascade_shadow(1, far_coord, 0.0002), blend);
    }
    if in_cascade(far_coord) {
        return cascade_shadow(1, far_coord, 0.0002);
    }
    return 1.0;
}

// 0 = something is directly overhead (roof, canopy), 1 = open sky. A wide,
// blurred lookup in a depth map rendered from straight above.
fn sky_visibility(world_pos: vec3<f32>, normal: vec3<f32>) -> f32 {
    let c = cascade_coord(globals.sky_view_proj, world_pos + normal * 0.25 + vec3<f32>(0.0, 0.1, 0.0));
    if any(c.xy < vec2<f32>(0.0)) || any(c.xy > vec2<f32>(1.0)) {
        return 1.0;
    }
    let texel = 1.0 / vec2<f32>(textureDimensions(sky_occlusion_map));
    var sum = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let offset = vec2<f32>(f32(x), f32(y)) * texel * 3.0;
            sum += textureSampleCompareLevel(sky_occlusion_map, shadow_sampler, c.xy + offset, c.z);
        }
    }
    return sum / 9.0;
}

// Sun + ambient + specular for an opaque surface, with fog. `n` is the
// (normal-mapped) shading normal, `geo_n` the geometric normal.
fn shade(
    albedo: vec3<f32>,
    n: vec3<f32>,
    geo_n: vec3<f32>,
    world_pos: vec3<f32>,
    specular: f32,
    shininess: f32,
    occlusion: f32,
) -> vec3<f32> {
    let to_light = to_sun();
    let to_eye = normalize(globals.camera_pos.xyz - world_pos);
    let n_dot_l = max(dot(n, to_light), 0.0);
    let shadow = sun_visibility(world_pos, geo_n);

    // Ambient: sky light from above, bounced ground light from below,
    // reduced under roofs and canopies.
    let hemi = mix(globals.ground_ambient.rgb, globals.sky_ambient.rgb, n.y * 0.5 + 0.5);
    let ambient = hemi * occlusion * mix(0.38, 1.0, sky_visibility(world_pos, n));

    let half_vec = normalize(to_light + to_eye);
    let spec = specular * pow(max(dot(n, half_vec), 0.0), shininess) * (shininess + 8.0) / 25.0;

    let sun = globals.sun_color.rgb * n_dot_l * shadow;
    return apply_fog(albedo * (ambient + sun) + sun * spec, world_pos);
}

// ---------------------------------------------------------------------------
// Meshes
// ---------------------------------------------------------------------------

// Steps a view ray (in tangent space) down through the height field stored
// in the normal texture's alpha channel and returns the UV where it hits,
// so the texture appears to extrude from the surface.
fn parallax_uv(uv: vec2<f32>, view_ts: vec3<f32>, depth: f32, dx: vec2<f32>, dy: vec2<f32>) -> vec2<f32> {
    // More steps at grazing angles, where the ray travels further.
    let steps = i32(mix(32.0, 8.0, clamp(view_ts.z, 0.0, 1.0)));
    let layer = 1.0 / f32(steps);
    // Tangent X runs along +U; tangent Y (bitangent) along -V.
    let shift = vec2<f32>(-view_ts.x, view_ts.y) / max(view_ts.z, 0.2) * depth * layer;

    var cur_uv = uv;
    var cur_layer = 0.0;
    var surface = 1.0 - textureSampleGrad(normal_texture, material_sampler, cur_uv, dx, dy).a;
    var prev_uv = uv;
    var prev_surface = surface;
    for (var i = 0; i < steps; i++) {
        if cur_layer >= surface {
            break;
        }
        prev_uv = cur_uv;
        prev_surface = surface;
        cur_uv += shift;
        cur_layer += layer;
        surface = 1.0 - textureSampleGrad(normal_texture, material_sampler, cur_uv, dx, dy).a;
    }
    // Refine between the last two steps.
    let after = surface - cur_layer;
    let before = prev_surface - (cur_layer - layer);
    let w = clamp(after / (after - before + 1e-6), 0.0, 1.0);
    return mix(cur_uv, prev_uv, w);
}

@fragment
fn fs_main(in: VertexOut, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
    // Texture-space derivatives, taken up front (they need uniform control
    // flow) and reused for every sample so parallax-shifted UVs keep the
    // right mip level.
    let uv_dx = dpdx(in.uv);
    let uv_dy = dpdy(in.uv);

    // Faces are drawn from both sides (some models are double-sided), so
    // light the back of a face with the flipped normal.
    // Cut-out materials (leaf cards) keep their normals on both sides: they
    // point away from the leaf cluster, so the crown shades as one volume.
    var geo_n = normalize(in.normal);
    if !front_facing && material.shading.x <= 0.0 {
        geo_n = -geo_n;
    }
    let t_raw = in.tangent.xyz - geo_n * dot(geo_n, in.tangent.xyz);
    let has_tangent = dot(t_raw, t_raw) > 1e-8;
    let t = select(vec3<f32>(1.0, 0.0, 0.0), normalize(t_raw), has_tangent);
    let b = cross(geo_n, t) * in.tangent.w;
    let to_eye = normalize(globals.camera_pos.xyz - in.world_position);

    // Parallax occlusion mapping: find where the view ray hits the height field.
    var uv = in.uv;
    let parallax_depth = material.shading.w * globals.extra.x;
    if parallax_depth > 0.0 && has_tangent {
        let view_ts = vec3<f32>(dot(to_eye, t), dot(to_eye, b), dot(to_eye, geo_n));
        uv = parallax_uv(in.uv, view_ts, parallax_depth, uv_dx, uv_dy);
    }

    let texel = textureSampleGrad(base_texture, material_sampler, uv, uv_dx, uv_dy);
    let mapped = textureSampleGrad(normal_texture, material_sampler, uv, uv_dx, uv_dy).xyz * 2.0 - 1.0;

    var albedo = material.base_color * in.color * texel;
    if albedo.a < material.shading.x {
        discard;
    }

    // Break up flat colors with world-space noise (skipped when unused; the
    // condition is uniform per draw, so the branch is cheap).
    if material.detail.x > 0.0 {
        let fine = fbm(in.world_position * material.detail.y);
        albedo = vec4<f32>(albedo.rgb * (1.0 + (fine - 0.5) * 2.0 * material.detail.x), albedo.a);
    }
    if material.detail.z > 0.0 {
        let broad = fbm(in.world_position * 0.045 + vec3<f32>(5.0, 0.0, 5.0));
        albedo = vec4<f32>(albedo.rgb * (1.0 + (broad - 0.5) * 2.0 * material.detail.z), albedo.a);
    }

    // Normal mapping. With the default flat normal texture this leaves n as is.
    var n = geo_n;
    if has_tangent {
        n = normalize(t * mapped.x + b * mapped.y + geo_n * mapped.z);
    }

    // Darker close to the ground (cheap contact occlusion).
    let above_ground = in.world_position.y - terrain_height(in.world_position.xz);
    let occlusion = 1.0 - material.detail.w * (1.0 - smoothstep(0.0, 1.2, above_ground));

    let color = shade(albedo.rgb, n, geo_n, in.world_position, material.shading.y, material.shading.z, occlusion);
    return vec4<f32>(tonemap(color), 1.0);
}

// ---------------------------------------------------------------------------
// Terrain
// ---------------------------------------------------------------------------
//
// Five ground layers (grass, dirt, rock, sand, paving) from texture arrays,
// mixed by a splat map that covers the whole terrain. The layers' height
// maps make the transitions follow the texture detail (grass between
// cobbles, sand in rock cracks) instead of a plain cross-fade.

struct TerrainParams {
    // 1 / tile size (meters) of layers 0..3.
    inv_tile: vec4<f32>,
    // Specular strength of layers 0..3.
    specular: vec4<f32>,
    // x = 1 / tile of layer 4, y = layer 4 specular, z = macro color variation.
    extra: vec4<f32>,
};

@group(1) @binding(8) var terrain_albedo: texture_2d_array<f32>;
@group(1) @binding(9) var terrain_normal: texture_2d_array<f32>;
@group(1) @binding(10) var terrain_splat: texture_2d<f32>;
@group(1) @binding(11) var terrain_sampler: sampler;
@group(1) @binding(12) var<uniform> terrain: TerrainParams;

@fragment
fn fs_terrain(in: VertexOut) -> @location(0) vec4<f32> {
    let p = in.world_position.xz;
    let geo_n = normalize(in.normal);

    let splat = textureSample(terrain_splat, terrain_sampler, p / (2.0 * globals.extra.z) + 0.5);
    let inv_tile = array<f32, 5>(terrain.inv_tile.x, terrain.inv_tile.y, terrain.inv_tile.z, terrain.inv_tile.w, terrain.extra.x);
    let specs = array<f32, 5>(terrain.specular.x, terrain.specular.y, terrain.specular.z, terrain.specular.w, terrain.extra.y);
    var weights = array<f32, 5>(max(1.0 - splat.r - splat.g - splat.b - splat.a, 0.0), splat.r, splat.g, splat.b, splat.a);

    var albedos: array<vec3<f32>, 5>;
    var normals: array<vec3<f32>, 5>;
    var heights: array<f32, 5>;
    // Anti-tiling: each layer is also sampled rotated and at a larger scale,
    // and the two are mixed by a slow noise, so no repeat is visible from afar.
    let mix_noise = smoothstep(0.35, 0.65, fbm(vec3<f32>(p.x, 0.0, p.y) * 0.03 + vec3<f32>(3.0, 0.0, 1.0)));
    // More of the large-scale copy far away, where a repeat is most visible.
    let view_distance = length(in.world_position - globals.camera_pos.xyz);
    let far_mix = clamp(0.3 + mix_noise * 0.4 + smoothstep(30.0, 120.0, view_distance) * 0.3, 0.0, 0.9);
    for (var i = 0; i < 5; i++) {
        let uv = p * inv_tile[i];
        let uv_big = vec2<f32>(-uv.y, uv.x) * 0.37 + vec2<f32>(0.21, 0.67);
        albedos[i] = mix(
            textureSample(terrain_albedo, terrain_sampler, uv, i).rgb,
            textureSample(terrain_albedo, terrain_sampler, uv_big, i).rgb,
            far_mix,
        );
        let nm = textureSample(terrain_normal, terrain_sampler, uv, i);
        normals[i] = nm.xyz * 2.0 - 1.0;
        heights[i] = nm.a;
    }
    // Rock on cliffs: project the texture from the side as well, so steep
    // faces aren't smeared (triplanar).
    let side_w = abs(geo_n.xz) / max(abs(geo_n.x) + abs(geo_n.z) + abs(geo_n.y), 1e-4);
    let rock_x = textureSample(terrain_albedo, terrain_sampler, in.world_position.zy * inv_tile[2], 2).rgb;
    let rock_z = textureSample(terrain_albedo, terrain_sampler, in.world_position.xy * inv_tile[2], 2).rgb;
    albedos[2] = albedos[2] * (1.0 - side_w.x - side_w.y) + rock_x * side_w.x + rock_z * side_w.y;

    // Height-based blending: the highest layer (weight + height) wins, with
    // a soft band below it.
    var best = 0.0;
    for (var i = 0; i < 5; i++) {
        weights[i] = weights[i] * (0.35 + heights[i]);
        best = max(best, weights[i]);
    }
    var total = 0.0;
    var albedo = vec3<f32>(0.0);
    var nts = vec3<f32>(0.0);
    var specular = 0.0;
    for (var i = 0; i < 5; i++) {
        let w = max(weights[i] - best + 0.22, 0.0);
        total += w;
        albedo += albedos[i] * w;
        nts += normals[i] * w;
        specular += specs[i] * w;
    }
    albedo /= total;
    specular /= total;

    // Large-scale color variation so the land doesn't look tiled.
    let broad = fbm(vec3<f32>(p.x, 0.0, p.y) * 0.045 + vec3<f32>(5.0, 0.0, 5.0));
    let wide = fbm(vec3<f32>(p.x, 0.0, p.y) * 0.008 + vec3<f32>(1.0, 0.0, 9.0));
    albedo *= (1.0 + (broad - 0.5) * 2.0 * terrain.extra.z) * (0.85 + 0.3 * wide);

    // Normal mapping with a world-aligned tangent frame (U along +X, V along +Z).
    let t = normalize(vec3<f32>(1.0, 0.0, 0.0) - geo_n * geo_n.x);
    let b = cross(geo_n, t);
    var n = normalize(t * nts.x + b * nts.y + geo_n * max(nts.z, 0.2));

    // Snow on the high peaks, settling on ledges but not on sheer faces.
    let snow_line = 58.0 + (broad - 0.5) * 14.0 + (wide - 0.5) * 10.0;
    let snow = smoothstep(snow_line - 5.0, snow_line + 3.0, in.world_position.y) * smoothstep(0.45, 0.7, geo_n.y);
    albedo = mix(albedo, vec3<f32>(0.9, 0.92, 0.96), snow);
    specular = mix(specular, 0.35, snow);
    n = normalize(mix(n, geo_n, snow * 0.7));

    let color = shade(albedo, n, geo_n, in.world_position, specular, 16.0, 1.0);
    return vec4<f32>(tonemap(color), 1.0);
}

// ---------------------------------------------------------------------------
// Water
// ---------------------------------------------------------------------------
//
// One flat sheet at the water level over the whole map; it only shows where
// the terrain dips below it. Depth (water level minus terrain height) drives
// color, transparency and shoreline foam.

struct WaterOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
};

@vertex
fn vs_water(@builtin(vertex_index) index: u32) -> WaterOut {
    let corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(1.0, -1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, 1.0),
    );
    let xz = corners[index] * globals.extra.z;
    let world = vec3<f32>(xz.x, globals.extra.y, xz.y);
    var out: WaterOut;
    out.clip_position = globals.view_proj * vec4<f32>(world, 1.0);
    out.world_position = world;
    return out;
}

// Small rolling waves: three layers of drifting noise.
fn wave_height(p: vec2<f32>, time: f32) -> f32 {
    return value_noise(vec3<f32>(p * 0.35 + vec2<f32>(time * 0.08, time * 0.05), time * 0.12)) * 0.6
        + value_noise(vec3<f32>(p * 1.1 - vec2<f32>(time * 0.15, time * 0.04), time * 0.25)) * 0.3
        + value_noise(vec3<f32>(p * 3.3 + vec2<f32>(time * 0.2, -time * 0.17), 3.0)) * 0.1;
}

@fragment
fn fs_water(in: WaterOut) -> @location(0) vec4<f32> {
    let p = in.world_position.xz;
    let depth = globals.extra.y - terrain_height(p);
    if depth <= 0.0 {
        discard;
    }
    let time = globals.camera_pos.w;
    let to_eye_vec = globals.camera_pos.xyz - in.world_position;
    let distance = length(to_eye_vec);
    let to_eye = to_eye_vec / distance;

    // Wave normal from finite differences; calmer far away to avoid shimmer.
    let e = 0.08;
    let h0 = wave_height(p, time);
    let hx = wave_height(p + vec2<f32>(e, 0.0), time);
    let hz = wave_height(p + vec2<f32>(0.0, e), time);
    let strength = 0.32 * (1.0 - smoothstep(25.0, 140.0, distance));
    let n = normalize(vec3<f32>(-(hx - h0) / e * strength, 1.0, -(hz - h0) / e * strength));

    let to_light = to_sun();
    let fresnel = 0.02 + 0.98 * pow(1.0 - max(dot(n, to_eye), 0.0), 5.0);
    let reflected = reflect(-to_eye, n);
    // Reflections are toned down and tinted so the lake stays readable at
    // grazing angles instead of matching the hazy horizon exactly.
    let reflection = sky_base(vec3<f32>(reflected.x, abs(reflected.y), reflected.z)) * vec3<f32>(0.78, 0.86, 0.95);
    let glint = pow(max(dot(n, normalize(to_light + to_eye)), 0.0), 500.0) * 18.0;

    // Clear and green in the shallows, deep blue further out.
    let lit = globals.sky_ambient.rgb * 0.9 + globals.sun_color.rgb * 0.3 * max(to_light.y, 0.0);
    let body = mix(vec3<f32>(0.10, 0.30, 0.27), vec3<f32>(0.012, 0.05, 0.08), smoothstep(0.0, 3.5, depth)) * lit;
    var color = mix(body, reflection, min(fresnel, 0.65)) + globals.sun_color.rgb * glint;
    var alpha = max(mix(0.25, 0.93, smoothstep(0.0, 2.2, depth)), fresnel);

    // Foam where the water meets the shore.
    let ripple = value_noise(vec3<f32>(p * 1.7, time * 0.4));
    let foam = (1.0 - smoothstep(0.0, 0.22, depth + (ripple - 0.5) * 0.16)) * 0.75;
    color = mix(color, vec3<f32>(0.92, 0.95, 0.96) * (globals.sky_ambient.rgb + globals.sun_color.rgb * 0.4), foam);
    alpha = max(alpha, foam);

    color = apply_fog(color, in.world_position);
    return vec4<f32>(tonemap(color), alpha);
}

// ---------------------------------------------------------------------------
// 3D grass and wildflowers
// ---------------------------------------------------------------------------
//
// Blades are generated entirely on the GPU: each instance is one blade on a
// grid that follows the camera (snapped to world cells, so blades don't
// slide as you move), with a random offset, height, facing and lean per
// cell, standing on the terrain. A mask texture says where grass grows and
// how many blades are flowers.

struct Grass {
    // x = half world size, y = blade spacing, z = draw radius, w = blades per grid side.
    params: vec4<f32>,
    // x = min height, y = max height, z = blade width, w = wind strength.
    shape: vec4<f32>,
    // x = ground macro color variation (to match the ground texture).
    ground: vec4<f32>,
};

@group(1) @binding(4) var<uniform> grass: Grass;
@group(1) @binding(5) var grass_mask: texture_2d<f32>;
@group(1) @binding(6) var grass_albedo: texture_2d<f32>;
@group(1) @binding(7) var grass_sampler: sampler;

struct GrassOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
    // 0 at the root, 1 at the tip.
    @location(3) along: f32,
};

// Random 0..1 per grid cell; `k` picks an independent value.
fn cell_random(cell: vec2<i32>, k: u32) -> f32 {
    let h = pcg(bitcast<u32>(cell.x) * 1597334677u ^ pcg(bitcast<u32>(cell.y) ^ (k * 2654435761u)));
    return f32(h) / 4294967295.0;
}

fn flower_color(pick: f32) -> vec3<f32> {
    if pick < 0.3 {
        return vec3<f32>(0.55, 0.28, 0.85); // violet
    } else if pick < 0.55 {
        return vec3<f32>(1.0, 0.82, 0.15); // buttercup yellow
    } else if pick < 0.8 {
        return vec3<f32>(0.95, 0.95, 0.9); // daisy white
    }
    return vec3<f32>(0.95, 0.35, 0.45); // pink
}

@vertex
fn vs_grass(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> GrassOut {
    var out: GrassOut;
    // Collapsed default: every vertex at the same clipped-away point, so
    // culled blades produce no pixels.
    out.clip_position = vec4<f32>(0.0, 0.0, -1.0, 1.0);

    let side_count = u32(grass.params.w);
    let spacing = grass.params.y;
    let radius = grass.params.z;
    let half_size = grass.params.x;
    let camera = globals.camera_pos.xz;

    let origin = vec2<i32>(floor(camera / spacing)) - vec2<i32>(i32(side_count / 2u));
    let cell = origin + vec2<i32>(i32(instance % side_count), i32(instance / side_count));
    let root = (vec2<f32>(cell) + vec2<f32>(cell_random(cell, 0u), cell_random(cell, 1u))) * spacing;

    let distance = length(root - camera);
    if distance > radius || abs(root.x) > half_size || abs(root.y) > half_size {
        return out;
    }
    let mask = textureSampleLevel(grass_mask, grass_sampler, root / (2.0 * half_size) + 0.5, 0.0);
    // Fewer (but wider) blades further away.
    let far = smoothstep(radius * 0.3, radius, distance);
    if cell_random(cell, 2u) > mask.r * mix(1.0, 0.35, far) {
        return out;
    }
    let is_flower = cell_random(cell, 8u) < mask.g;

    // Blade shape: 7 vertices as a triangle strip, two per level plus the tip.
    let fade = 1.0 - smoothstep(radius * 0.45, radius, distance);
    var height = mix(grass.shape.x, grass.shape.y, cell_random(cell, 3u)) * fade * (0.5 + 0.5 * mask.r);
    var width = grass.shape.z * mix(1.0, 2.2, far) * mix(0.8, 1.2, cell_random(cell, 6u));
    let yaw = cell_random(cell, 4u) * 6.2831853;
    let facing = vec2<f32>(cos(yaw), sin(yaw));
    let right = vec3<f32>(-facing.y, 0.0, facing.x);
    let level = vertex / 2u;
    let t = f32(level) / 3.0;
    let side = f32(vertex % 2u) - 0.5;

    // Width profile: tapering blade, or a thin stem with a bloom on top.
    var profile = 1.0 - pow(t, 1.5);
    if is_flower {
        height = height * 0.85 + 0.08;
        profile = select(0.35, 1.7, level == 2u);
    }

    // Natural lean plus wind gusts that roll across the field.
    let time = globals.camera_pos.w;
    let gust = sin(time * 1.3 + root.x * 0.25 + root.y * 0.18) * 0.5
        + sin(time * 2.9 + root.x * 0.9 - root.y * 0.7) * 0.2 + 0.35;
    let wind_dir = normalize(vec2<f32>(1.0, 0.4));
    let lean = 0.15 + 0.35 * cell_random(cell, 5u);
    let bend = (facing * lean + wind_dir * gust * grass.shape.w) * t * t * height;

    let ground = terrain_height(root);
    let position = vec3<f32>(root.x, ground, root.y)
        + right * side * width * profile
        + vec3<f32>(bend.x, height * t * (1.0 - 0.15 * t * lean), bend.y);

    // Color: the ground texture under the blade (so near and far grass
    // match), darker at the root and lighter, yellower at the tip.
    let ground_uv = root / 4.0;
    var base = textureSampleLevel(grass_albedo, grass_sampler, ground_uv, 2.0).rgb;
    let broad = fbm(vec3<f32>(root.x, 0.0, root.y) * 0.045 + vec3<f32>(5.0, 0.0, 5.0));
    base *= (1.0 + (broad - 0.5) * 2.0 * grass.ground.x) * mix(0.85, 1.2, cell_random(cell, 7u));
    var tip = base * 1.45 + vec3<f32>(0.025, 0.02, 0.0);
    var color = mix(base * 0.55, tip, t);
    if is_flower && level >= 2u {
        color = flower_color(cell_random(cell, 9u)) * mix(0.8, 1.1, cell_random(cell, 10u));
    }

    out.clip_position = globals.view_proj * vec4<f32>(position, 1.0);
    out.world_position = position;
    out.normal = normalize(vec3<f32>(facing.x, 0.3 + t, facing.y));
    out.color = color;
    out.along = t;
    return out;
}

@fragment
fn fs_grass(in: GrassOut, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
    var n = normalize(in.normal);
    if !front_facing {
        n = -n;
    }
    // Thin blades: mostly lit like the ground they stand on.
    n = normalize(mix(n, vec3<f32>(0.0, 1.0, 0.0), 0.55));

    let to_light = to_sun();
    let to_eye = normalize(globals.camera_pos.xyz - in.world_position);
    let up = vec3<f32>(0.0, 1.0, 0.0);
    let shadow = sun_visibility(in.world_position, up);
    let sky_open = mix(0.38, 1.0, sky_visibility(in.world_position, up));

    // Wrapped diffuse plus light shining through the blade towards the eye.
    let diffuse = clamp(dot(n, to_light) * 0.6 + 0.4, 0.0, 1.0);
    let through = pow(max(dot(-to_eye, to_light), 0.0), 4.0) * 0.6 * in.along;
    let hemi = mix(globals.ground_ambient.rgb, globals.sky_ambient.rgb, n.y * 0.5 + 0.5);
    let root_occlusion = mix(0.5, 1.0, in.along);

    let color = in.color * (hemi * root_occlusion * sky_open + globals.sun_color.rgb * (diffuse + through) * shadow);
    return vec4<f32>(tonemap(apply_fog(color, in.world_position)), 1.0);
}
