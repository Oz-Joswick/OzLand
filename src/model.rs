//! Loading 3D models from files (glTF 2.0 `.gltf`/`.glb`, Wavefront `.obj`).
//!
//! A loaded [`Model`] is a list of parts, one per mesh primitive, each with
//! its own [`MaterialData`]. Node transforms from the file are baked into
//! the vertices, so every part is in the model's own coordinate space.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use glam::{Mat4, Vec3, Vec4};

use crate::collision::Aabb;
use crate::mesh::{DEFAULT_TANGENT, MeshData, Vertex};

/// An RGBA8 image.
pub struct TextureData {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// True for color images (sRGB-encoded); false for data such as normal maps.
    pub srgb: bool,
}

#[derive(Clone)]
pub struct MaterialData {
    /// Linear RGBA, multiplied with the texture and vertex color.
    pub base_color: Vec4,
    pub base_color_texture: Option<Arc<TextureData>>,
    /// Tangent-space normal map (requires vertex tangents).
    pub normal_texture: Option<Arc<TextureData>>,
    /// Fragments with alpha below this are discarded (glTF `MASK` mode).
    /// 0 disables the test.
    pub alpha_cutoff: f32,
    /// Strength and tightness of the sun's specular highlight.
    pub specular: f32,
    pub shininess: f32,
    /// World-space noise that breaks up flat colors (0 = off) and its
    /// frequency in cycles per meter.
    pub detail_noise: f32,
    pub detail_scale: f32,
    /// Very low-frequency color variation, to hide texture tiling on big surfaces.
    pub macro_variation: f32,
    /// How much darker surfaces get close to the ground (cheap ambient occlusion).
    pub ground_occlusion: f32,
    /// Parallax occlusion mapping: how deep (in texture UV units) the
    /// height map in the normal texture's alpha channel appears to extrude
    /// the surface. 0 disables it. E.g. 0.02 on a texture that repeats every
    /// 2 m gives 4 cm of relief.
    pub parallax_depth: f32,
}

impl Default for MaterialData {
    fn default() -> Self {
        Self {
            base_color: Vec4::ONE,
            base_color_texture: None,
            normal_texture: None,
            alpha_cutoff: 0.0,
            specular: 0.2,
            shininess: 24.0,
            detail_noise: 0.0,
            detail_scale: 3.0,
            macro_variation: 0.0,
            ground_occlusion: 0.35,
            parallax_depth: 0.0,
        }
    }
}

impl MaterialData {
    /// Untextured materials from model files get world-space detail noise so
    /// they don't render as one flat color.
    fn with_auto_detail(mut self) -> Self {
        if self.base_color_texture.is_none() {
            self.detail_noise = 0.6;
            self.detail_scale = 4.0;
        }
        self
    }
}

pub struct ModelPart {
    pub mesh: MeshData,
    pub material: MaterialData,
}

pub struct Model {
    pub parts: Vec<ModelPart>,
    pub bounds: Aabb,
}

/// Loads a model, picking the loader from the file extension.
pub fn load(path: &Path) -> Result<Model> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    let parts = match ext.as_str() {
        "gltf" | "glb" => load_gltf(path),
        "obj" => load_obj(path),
        _ => bail!("unsupported model format '.{ext}' (use .glb, .gltf or .obj)"),
    }
    .with_context(|| format!("loading model {}", path.display()))?;

    let bounds = parts
        .iter()
        .filter_map(|p| p.mesh.bounds())
        .reduce(|a, b| a.union(&b))
        .with_context(|| format!("model {} contains no triangles", path.display()))?;
    Ok(Model { parts, bounds })
}

/// The `assets` folder: next to the current directory when run with
/// `cargo run`, otherwise the one in the source tree.
pub fn asset_root() -> PathBuf {
    let local = PathBuf::from("assets");
    // A released copy: assets sit next to the executable.
    let beside_exe = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("assets")));
    if local.is_dir() {
        local
    } else if let Some(dir) = beside_exe.filter(|d| d.is_dir()) {
        dir
    } else {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("assets")
    }
}

// ---------------------------------------------------------------------------
// glTF
// ---------------------------------------------------------------------------

fn load_gltf(path: &Path) -> Result<Vec<ModelPart>> {
    let (doc, buffers, images) = gltf::import(path)?;

    // The same image can't be shared between color and normal-map use: one
    // is sRGB-encoded, the other is linear data.
    let texture = |index: usize, srgb: bool| -> Option<Arc<TextureData>> {
        let mut data = gltf_image_to_rgba(&images[index])?;
        data.srgb = srgb;
        Some(Arc::new(data))
    };

    let materials: Vec<MaterialData> = doc
        .materials()
        .map(|m| {
            let pbr = m.pbr_metallic_roughness();
            let roughness = pbr.roughness_factor();
            MaterialData {
                base_color: Vec4::from(pbr.base_color_factor()),
                base_color_texture: pbr
                    .base_color_texture()
                    .and_then(|info| texture(info.texture().source().index(), true)),
                normal_texture: m
                    .normal_texture()
                    .and_then(|info| texture(info.texture().source().index(), false)),
                alpha_cutoff: match m.alpha_mode() {
                    gltf::material::AlphaMode::Mask => m.alpha_cutoff().unwrap_or(0.5),
                    // No sorting/blending: treat blended materials as cut-outs
                    // so fully transparent areas at least disappear.
                    gltf::material::AlphaMode::Blend => 0.05,
                    gltf::material::AlphaMode::Opaque => 0.0,
                },
                // Rough approximation of PBR roughness with Blinn-Phong.
                specular: 0.6 * (1.0 - roughness) + 0.05,
                shininess: 4.0 + 120.0 * (1.0 - roughness).powi(2),
                ..Default::default()
            }
            .with_auto_detail()
        })
        .collect();

    let scene = doc
        .default_scene()
        .or_else(|| doc.scenes().next())
        .context("file has no scene")?;

    let mut parts = Vec::new();
    let mut stack: Vec<(gltf::Node, Mat4)> = scene.nodes().map(|n| (n, Mat4::IDENTITY)).collect();
    while let Some((node, parent)) = stack.pop() {
        let world = parent * Mat4::from_cols_array_2d(&node.transform().matrix());
        if let Some(mesh) = node.mesh() {
            for prim in mesh.primitives() {
                if prim.mode() != gltf::mesh::Mode::Triangles {
                    log::warn!("{}: skipping non-triangle primitive ({:?})", path.display(), prim.mode());
                    continue;
                }
                let reader = prim.reader(|b| Some(&buffers[b.index()]));
                let Some(positions) = reader.read_positions() else {
                    continue;
                };
                let positions: Vec<[f32; 3]> = positions.collect();
                let normals: Option<Vec<[f32; 3]>> = reader.read_normals().map(|n| n.collect());
                let uvs: Option<Vec<[f32; 2]>> = reader.read_tex_coords(0).map(|t| t.into_f32().collect());
                let colors: Option<Vec<[f32; 4]>> = reader.read_colors(0).map(|c| c.into_rgba_f32().collect());
                let tangents: Option<Vec<[f32; 4]>> = reader.read_tangents().map(|t| t.collect());

                let vertices = (0..positions.len())
                    .map(|i| Vertex {
                        position: positions[i],
                        normal: normals.as_ref().map_or([0.0, 1.0, 0.0], |n| n[i]),
                        uv: uvs.as_ref().map_or([0.0, 0.0], |t| t[i]),
                        color: colors.as_ref().map_or([1.0; 4], |c| c[i]),
                        tangent: tangents.as_ref().map_or(DEFAULT_TANGENT, |t| t[i]),
                    })
                    .collect();
                let indices = match reader.read_indices() {
                    Some(i) => i.into_u32().collect(),
                    None => (0..positions.len() as u32).collect(),
                };

                let material = prim
                    .material()
                    .index()
                    .map(|i| materials[i].clone())
                    .unwrap_or_else(|| MaterialData::default().with_auto_detail());

                let mut mesh_data = MeshData { vertices, indices };
                if normals.is_none() {
                    mesh_data.compute_flat_normals();
                }
                if material.normal_texture.is_some() && (tangents.is_none() || normals.is_none()) {
                    mesh_data.compute_tangents();
                }
                mesh_data.transform(world);
                parts.push(ModelPart { mesh: mesh_data, material });
            }
        }
        stack.extend(node.children().map(|c| (c, world)));
    }
    Ok(parts)
}

fn gltf_image_to_rgba(img: &gltf::image::Data) -> Option<TextureData> {
    use gltf::image::Format;
    let px = &img.pixels;
    let rgba: Vec<u8> = match img.format {
        Format::R8G8B8A8 => px.clone(),
        Format::R8G8B8 => px.chunks_exact(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        Format::R8G8 => px.chunks_exact(2).flat_map(|c| [c[0], c[0], c[0], c[1]]).collect(),
        Format::R8 => px.iter().flat_map(|&c| [c, c, c, 255]).collect(),
        other => {
            log::warn!("unsupported texture format {other:?}; ignoring texture");
            return None;
        }
    };
    Some(TextureData {
        width: img.width,
        height: img.height,
        rgba,
        srgb: true,
    })
}

// ---------------------------------------------------------------------------
// OBJ
// ---------------------------------------------------------------------------

fn load_obj(path: &Path) -> Result<Vec<ModelPart>> {
    let options = tobj::LoadOptions {
        single_index: true,
        triangulate: true,
        ignore_points: true,
        ignore_lines: true,
    };
    let (models, materials) = tobj::load_obj(path, &options)?;
    let materials = materials.unwrap_or_else(|e| {
        log::warn!("{}: could not load materials: {e}", path.display());
        Vec::new()
    });
    let base_dir = path.parent().unwrap_or(Path::new("."));

    let materials: Vec<MaterialData> = materials
        .iter()
        .map(|m| {
            let color = m.diffuse.unwrap_or([1.0; 3]);
            let texture = m.diffuse_texture.as_ref().and_then(|file| {
                let tex_path = base_dir.join(file);
                match image::open(&tex_path) {
                    Ok(img) => {
                        let img = img.to_rgba8();
                        Some(Arc::new(TextureData {
                            width: img.width(),
                            height: img.height(),
                            rgba: img.into_raw(),
                            srgb: true,
                        }))
                    }
                    Err(e) => {
                        log::warn!("could not load texture {}: {e}", tex_path.display());
                        None
                    }
                }
            });
            MaterialData {
                base_color: Vec3::from(color).extend(m.dissolve.unwrap_or(1.0)),
                base_color_texture: texture,
                ..Default::default()
            }
            .with_auto_detail()
        })
        .collect();

    let mut parts = Vec::new();
    for model in models {
        let m = &model.mesh;
        let count = m.positions.len() / 3;
        let vertices = (0..count)
            .map(|i| Vertex {
                position: [m.positions[3 * i], m.positions[3 * i + 1], m.positions[3 * i + 2]],
                normal: if m.normals.is_empty() {
                    [0.0, 1.0, 0.0]
                } else {
                    [m.normals[3 * i], m.normals[3 * i + 1], m.normals[3 * i + 2]]
                },
                // OBJ puts the texture origin at the bottom-left; wgpu at the top-left.
                uv: if m.texcoords.is_empty() {
                    [0.0, 0.0]
                } else {
                    [m.texcoords[2 * i], 1.0 - m.texcoords[2 * i + 1]]
                },
                color: if m.vertex_color.is_empty() {
                    [1.0; 4]
                } else {
                    [m.vertex_color[3 * i], m.vertex_color[3 * i + 1], m.vertex_color[3 * i + 2], 1.0]
                },
                tangent: DEFAULT_TANGENT,
            })
            .collect();
        let mut mesh = MeshData {
            vertices,
            indices: m.indices.clone(),
        };
        if m.normals.is_empty() {
            mesh.compute_flat_normals();
        }
        let material = m
            .material_id
            .and_then(|id| materials.get(id).cloned())
            .unwrap_or_else(|| MaterialData::default().with_auto_detail());
        parts.push(ModelPart { mesh, material });
    }
    Ok(parts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_sample_gltf() {
        let model = load(&asset_root().join("models/pine_tree.glb")).unwrap();
        assert_eq!(model.parts.len(), 2, "trunk + foliage");
        let size = model.bounds.size();
        assert!(size.y > 5.0 && size.y < 6.0, "height {}", size.y);
        for part in &model.parts {
            assert!(!part.mesh.is_empty());
            assert!(part.mesh.indices.iter().all(|&i| (i as usize) < part.mesh.vertices.len()));
            for v in &part.mesh.vertices {
                let len = Vec3::from(v.normal).length();
                assert!((len - 1.0).abs() < 1e-3, "normal not unit length: {len}");
            }
        }
        // The two materials have different colors.
        assert_ne!(model.parts[0].material.base_color, model.parts[1].material.base_color);
    }

    #[test]
    fn loads_sample_obj() {
        let model = load(&asset_root().join("models/rock.obj")).unwrap();
        assert_eq!(model.parts.len(), 1);
        let part = &model.parts[0];
        assert_eq!(part.mesh.indices.len(), 80 * 3);
        // Kd from rock.mtl was applied (not the white default).
        assert!(part.material.base_color.x < 0.5);
        // Every face's stored normal points away from the rock's center.
        for tri in part.mesh.indices.chunks_exact(3) {
            let v = part.mesh.vertices[tri[0] as usize];
            assert!(Vec3::from(v.normal).dot(Vec3::from(v.position)) > 0.0);
        }
    }

    /// A third-party file: node hierarchy, several meshes, textures.
    #[test]
    fn loads_textured_gltf() {
        let model = load(&asset_root().join("models/lantern/Lantern.glb")).unwrap();
        assert!(model.parts.len() >= 3);
        let tex = model.parts[0].material.base_color_texture.as_ref().expect("base color texture");
        assert_eq!(tex.rgba.len(), (tex.width * tex.height * 4) as usize);
        assert!(model.parts.iter().all(|p| p.mesh.vertices.iter().any(|v| v.uv != [0.0, 0.0])));
    }

    #[test]
    fn rejects_unknown_format() {
        assert!(load(Path::new("model.fbx")).is_err());
    }
}
