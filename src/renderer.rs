//! wgpu renderer. Each frame:
//!
//! 1. **Depth-only passes** (only when something changed): two sun shadow
//!    cascades (a sharp one around the player, a coarse one over the whole
//!    world) and a top-down sky-occlusion map.
//! 2. **Main pass** (4x MSAA, reverse-Z depth): procedural sky, terrain,
//!    meshes, 3D grass, then water (blended).
//! 3. **UI pass**: egui on top.
//!
//! Each [`Batch`] is one mesh (vertex + index buffer), one material (bind
//! group) and a buffer of per-instance model matrices and tints. The
//! terrain heightmap is bound with the globals, so every shader can ask for
//! the ground height under any point.

use std::sync::Arc;

use anyhow::{Context, Result};
use bytemuck::{Pod, Zeroable};
use glam::{Mat3, Mat4, Vec3, Vec4, camera};
use wgpu::util::DeviceExt;
use winit::window::Window;

use crate::camera::Camera;
use crate::mesh::{MeshData, Vertex};
use crate::model::{MaterialData, TextureData};
use crate::textures::SurfaceTextures;

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const SAMPLE_COUNT: u32 = 4;
const SHADOW_MAP_SIZE: u32 = 4096;
/// Shadow cascades: 0 = sharp, around the player; 1 = coarse, whole world.
const CASCADES: u32 = 2;
/// The near cascade is re-centred when the player moves this far.
const NEAR_CASCADE_SNAP: f32 = 8.0;
/// Top-down depth map that tells which surfaces are sheltered from the sky.
const SKY_MAP_SIZE: u32 = 2048;

/// Scene-wide lighting and atmosphere settings. Colors are linear RGB and
/// may exceed 1 (they are tone-mapped at the end).
pub struct Lighting {
    /// Direction the sunlight travels (pointing down towards the ground).
    pub sun_direction: Vec3,
    pub sun_color: Vec3,
    pub sky_zenith: Vec3,
    pub sky_horizon: Vec3,
    /// Ambient light arriving from the sky (lights upward-facing surfaces).
    pub sky_ambient: Vec3,
    /// Ambient light bounced off the ground (lights downward-facing surfaces).
    pub ground_ambient: Vec3,
    pub fog_start: f32,
    pub fog_end: f32,
    pub exposure: f32,
    /// Radius of the sharp shadow cascade around the player.
    pub shadow_near_radius: f32,
    /// The world spans -half..half on X and Z, and this height range.
    pub world_half_size: f32,
    pub world_height_range: (f32, f32),
}

/// 3D grass blades over the ground, generated on the GPU around the camera.
pub struct GrassField {
    /// Red channel: grass density (1 = full, 0 = none). Green: chance a
    /// blade is a wildflower. Covers -half_size..half_size on X and Z.
    pub mask: TextureData,
    /// The ground grass texture (repeating every 4 m), used to color blades.
    pub albedo: Arc<TextureData>,
    pub half_size: f32,
    /// Same as the ground's macro variation, so colors match.
    pub macro_variation: f32,
    /// Distance between blades (meters) and how far from the camera they're drawn.
    pub spacing: f32,
    pub radius: f32,
    pub min_height: f32,
    pub max_height: f32,
    pub blade_width: f32,
    pub wind: f32,
}

/// Everything needed to draw the terrain.
pub struct TerrainData<'a> {
    pub mesh: &'a MeshData,
    /// Row-major heights, `res` x `res`, covering -half_size..half_size.
    pub heights: &'a [f32],
    pub res: u32,
    pub half_size: f32,
    pub cell: f32,
    pub water_level: f32,
    /// Ground layers in splat order: grass, dirt, rock, sand, paving.
    pub layers: [&'a SurfaceTextures; 5],
    /// How many meters each layer's texture covers before repeating.
    pub tile_meters: [f32; 5],
    pub specular: [f32; 5],
    /// RGBA weights for dirt, rock, sand, paving (grass is what's left),
    /// covering the whole terrain.
    pub splat: &'a TextureData,
    pub macro_variation: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GrassUniform {
    params: [f32; 4],
    shape: [f32; 4],
    ground: [f32; 4],
}

struct GrassDraw {
    bind_group: wgpu::BindGroup,
    buffer: wgpu::Buffer,
    uniform: GrassUniform,
    /// Spacing from the scene; menu settings scale it.
    base_spacing: f32,
    enabled: bool,
    instances: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct TerrainUniform {
    /// 1 / tile size per layer (0..3), then layer 4 in `extra.x`.
    inv_tile: [f32; 4],
    specular: [f32; 4],
    /// x = 1 / tile of layer 4, y = layer 4 specular, z = macro variation.
    extra: [f32; 4],
}

struct TerrainDraw {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    instance_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

/// The UI to draw over the frame this frame (from egui).
pub struct UiFrame {
    pub primitives: Vec<egui::ClippedPrimitive>,
    pub textures_delta: egui::TexturesDelta,
    pub pixels_per_point: f32,
}

/// One placement of a mesh.
#[derive(Clone, Copy)]
pub struct Instance {
    pub transform: Mat4,
    /// Linear RGBA multiplied with the material color.
    pub tint: Vec4,
}

impl Instance {
    pub const IDENTITY: Instance = Instance {
        transform: Mat4::IDENTITY,
        tint: Vec4::ONE,
    };
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GlobalsUniform {
    view_proj: [[f32; 4]; 4],
    inv_view_proj: [[f32; 4]; 4],
    /// The matrix used by depth-only passes; in the main globals it's the
    /// near shadow cascade.
    light_view_proj: [[f32; 4]; 4],
    far_light_view_proj: [[f32; 4]; 4],
    sky_view_proj: [[f32; 4]; 4],
    camera_pos: [f32; 4],
    sun_dir: [f32; 4],
    sun_color: [f32; 4],
    sky_zenith: [f32; 4],
    sky_horizon: [f32; 4],
    sky_ambient: [f32; 4],
    ground_ambient: [f32; 4],
    params: [f32; 4],
    // x = parallax scale, y = water level, z = terrain half size, w = terrain cell size.
    extra: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MaterialUniform {
    base_color: [f32; 4],
    shading: [f32; 4],
    detail: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct InstanceRaw {
    model: [[f32; 4]; 4],
    normal: [[f32; 3]; 3],
    tint: [f32; 4],
}

impl InstanceRaw {
    fn new(instance: &Instance) -> Self {
        let normal = Mat3::from_mat4(instance.transform).inverse().transpose();
        Self {
            model: instance.transform.to_cols_array_2d(),
            normal: normal.to_cols_array_2d(),
            tint: instance.tint.to_array(),
        }
    }
}

const VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
    0 => Float32x3, // position
    1 => Float32x3, // normal
    2 => Float32x2, // uv
    3 => Float32x4, // color
    4 => Float32x4, // tangent
];

const INSTANCE_ATTRIBUTES: [wgpu::VertexAttribute; 8] = wgpu::vertex_attr_array![
    5 => Float32x4, 6 => Float32x4, 7 => Float32x4, 8 => Float32x4, // model matrix
    9 => Float32x3, 10 => Float32x3, 11 => Float32x3,               // normal matrix
    12 => Float32x4,                                                  // tint
];

fn vertex_layouts() -> [Option<wgpu::VertexBufferLayout<'static>>; 2] {
    [
        Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &VERTEX_ATTRIBUTES,
        }),
        Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<InstanceRaw>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &INSTANCE_ATTRIBUTES,
        }),
    ]
}

struct Batch {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    instance_buffer: wgpu::Buffer,
    instance_count: u32,
    material: wgpu::BindGroup,
    /// Drawn in the main pass / into the shadow maps.
    visible: bool,
    casts_shadow: bool,
}

/// A uniform buffer with the globals layout, and its bind group.
struct GlobalsSlot {
    buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    main_pipeline: wgpu::RenderPipeline,
    sky_pipeline: wgpu::RenderPipeline,
    shadow_pipeline: wgpu::RenderPipeline,
    grass_pipeline: wgpu::RenderPipeline,
    terrain_pipeline: wgpu::RenderPipeline,
    water_pipeline: wgpu::RenderPipeline,
    globals_layout: wgpu::BindGroupLayout,
    grass_layout: wgpu::BindGroupLayout,
    terrain_layout: wgpu::BindGroupLayout,
    material_layout: wgpu::BindGroupLayout,
    grass: Option<GrassDraw>,
    terrain: Option<TerrainDraw>,
    /// Main globals (also the near shadow cascade pass), far cascade pass,
    /// sky-occlusion pass.
    globals: [GlobalsSlot; 3],
    height_map: wgpu::TextureView,
    terrain_params: [f32; 3],
    shadow_bind_group: wgpu::BindGroup,
    shadow_layer_views: Vec<wgpu::TextureView>,
    sky_occlusion_view: wgpu::TextureView,
    sampler: wgpu::Sampler,
    white_texture: wgpu::TextureView,
    flat_normal_texture: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    msaa_view: wgpu::TextureView,
    batches: Vec<Batch>,
    screenshot: Option<std::path::PathBuf>,
    /// Scene or sun changed: re-render every depth map.
    world_maps_dirty: bool,
    /// Where the near cascade was last rendered.
    near_cascade_center: Option<Vec3>,
    parallax_scale: f32,
    ui_renderer: egui_wgpu::Renderer,
    /// An opaque, untextured material, bound when drawing the terrain into
    /// the shadow maps.
    plain_material: Option<wgpu::BindGroup>,
}

fn uniform_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn texture_entry(binding: u32, sample_type: wgpu::TextureSampleType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn sampler_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

fn depth_state(write: bool, compare: wgpu::CompareFunction) -> Option<wgpu::DepthStencilState> {
    Some(wgpu::DepthStencilState {
        format: DEPTH_FORMAT,
        depth_write_enabled: Some(write),
        depth_compare: Some(compare),
        stencil: Default::default(),
        bias: Default::default(),
    })
}

impl Renderer {
    pub fn new(window: Arc<Window>) -> Result<Self> {
        pollster::block_on(Self::new_async(window))
    }

    async fn new_async(window: Arc<Window>) -> Result<Self> {
        let size = window.inner_size();
        // Options can be overridden with WGPU_BACKEND etc. The display handle
        // is only needed by the OpenGL backend.
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle_from_env(
            Box::new(window.clone()),
        ));
        let surface = instance.create_surface(window)?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await
            .context("no suitable GPU adapter found")?;
        log::info!("using GPU adapter: {:?}", adapter.get_info());

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("device"),
                ..Default::default()
            })
            .await?;
        // Log GPU validation errors with their full message.
        device.on_uncaptured_error(Arc::new(|e| log::error!("wgpu: {e}")));

        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .context("surface is not supported by the adapter")?;
        // Prefer an sRGB surface so the shader can work in linear color.
        let caps = surface.get_capabilities(&adapter);
        if let Some(format) = caps.formats.iter().copied().find(|f| f.is_srgb()) {
            config.format = format;
        }
        config.present_mode = wgpu::PresentMode::AutoVsync;
        // Allows saving screenshots (see `request_screenshot`) where supported.
        if caps.usages.contains(wgpu::TextureUsages::COPY_SRC) {
            config.usage |= wgpu::TextureUsages::COPY_SRC;
        }
        surface.configure(&device, &config);

        // --- bind group layouts ---------------------------------------------
        let all_stages = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals layout"),
            entries: &[
                uniform_entry(0, all_stages),
                // Terrain heightmap (R32Float, read with textureLoad).
                wgpu::BindGroupLayoutEntry {
                    visibility: all_stages,
                    ..texture_entry(1, wgpu::TextureSampleType::Float { filterable: false })
                },
            ],
        });
        let float_texture = wgpu::TextureSampleType::Float { filterable: true };
        let material_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("material layout"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::FRAGMENT),
                texture_entry(1, float_texture),
                texture_entry(2, float_texture),
                sampler_entry(3, wgpu::ShaderStages::FRAGMENT),
            ],
        });
        let shadow_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
                texture_entry(2, wgpu::TextureSampleType::Depth),
            ],
        });
        // Group-1 layouts share one shader module, so each uses its own
        // binding numbers: material 0..3, grass 4..7, terrain 8..12.
        let grass_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("grass layout"),
            entries: &[
                uniform_entry(4, wgpu::ShaderStages::VERTEX),
                wgpu::BindGroupLayoutEntry {
                    visibility: wgpu::ShaderStages::VERTEX,
                    ..texture_entry(5, float_texture)
                },
                wgpu::BindGroupLayoutEntry {
                    visibility: wgpu::ShaderStages::VERTEX,
                    ..texture_entry(6, float_texture)
                },
                sampler_entry(7, wgpu::ShaderStages::VERTEX),
            ],
        });
        let array_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: float_texture,
                view_dimension: wgpu::TextureViewDimension::D2Array,
                multisampled: false,
            },
            count: None,
        };
        let terrain_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("terrain layout"),
            entries: &[
                array_entry(8),
                array_entry(9),
                texture_entry(10, float_texture),
                sampler_entry(11, wgpu::ShaderStages::FRAGMENT),
                uniform_entry(12, wgpu::ShaderStages::FRAGMENT),
            ],
        });

        // --- resources ------------------------------------------------------
        let height_map = upload_heightmap(&device, &queue, &[0.0; 4], 2);
        let make_globals = |label| {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: std::mem::size_of::<GlobalsUniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bind_group = globals_bind_group(&device, &globals_layout, &buffer, &height_map);
            GlobalsSlot { buffer, bind_group }
        };
        let globals = [
            make_globals("globals"),
            make_globals("far cascade globals"),
            make_globals("sky occlusion globals"),
        ];

        let shadow_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shadow cascades"),
            size: wgpu::Extent3d {
                width: SHADOW_MAP_SIZE,
                height: SHADOW_MAP_SIZE,
                depth_or_array_layers: CASCADES,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let shadow_layer_views = (0..CASCADES)
            .map(|layer| {
                shadow_texture.create_view(&wgpu::TextureViewDescriptor {
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_array_layer: layer,
                    array_layer_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        let shadow_array_view = shadow_texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let sky_occlusion_view = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("sky occlusion map"),
                size: wgpu::Extent3d {
                    width: SKY_MAP_SIZE,
                    height: SKY_MAP_SIZE,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: DEPTH_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default());
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shadow sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let shadow_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow"),
            layout: &shadow_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&shadow_array_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&shadow_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&sky_occlusion_view),
                },
            ],
        });

        // --- pipelines ------------------------------------------------------
        let shader = device.create_shader_module(wgpu::include_wgsl!("shader.wgsl"));
        let layout = |label, groups: &[Option<&wgpu::BindGroupLayout>]| {
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: groups,
                ..Default::default()
            })
        };
        let main_layout = layout("main layout", &[Some(&globals_layout), Some(&material_layout), Some(&shadow_layout)]);
        let grass_pipeline_layout =
            layout("grass layout", &[Some(&globals_layout), Some(&grass_layout), Some(&shadow_layout)]);
        let terrain_pipeline_layout =
            layout("terrain layout", &[Some(&globals_layout), Some(&terrain_layout), Some(&shadow_layout)]);
        let globals_only_layout = layout("globals-only layout", &[Some(&globals_layout)]);

        let opaque_target = [Some(wgpu::ColorTargetState {
            format: config.format,
            blend: Some(wgpu::BlendState::REPLACE),
            write_mask: wgpu::ColorWrites::ALL,
        })];
        let blended_target = [Some(wgpu::ColorTargetState {
            format: config.format,
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            write_mask: wgpu::ColorWrites::ALL,
        })];
        let msaa = wgpu::MultisampleState {
            count: SAMPLE_COUNT,
            mask: !0,
            alpha_to_coverage_enabled: false,
        };
        // Not culled: glTF models may be double-sided. The shader flips the
        // normal for back faces.
        let triangles = wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            ..Default::default()
        };
        let layouts = vertex_layouts();
        // Reverse-Z: near = 1, far = 0, so nearer means greater.
        let pipeline = |label,
                        layout: &wgpu::PipelineLayout,
                        vs,
                        fs,
                        buffers: &[Option<wgpu::VertexBufferLayout>],
                        primitive,
                        depth,
                        targets: &[Option<wgpu::ColorTargetState>]| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vs),
                    compilation_options: Default::default(),
                    buffers,
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets,
                }),
                primitive,
                depth_stencil: depth,
                multisample: msaa,
                multiview_mask: None,
                cache: None,
            })
        };
        use wgpu::CompareFunction::{Always, Greater};
        let main_pipeline = pipeline(
            "mesh pipeline",
            &main_layout,
            "vs_main",
            "fs_main",
            &layouts,
            triangles,
            depth_state(true, Greater),
            &opaque_target,
        );
        let terrain_pipeline = pipeline(
            "terrain pipeline",
            &terrain_pipeline_layout,
            "vs_main",
            "fs_terrain",
            &layouts,
            triangles,
            depth_state(true, Greater),
            &opaque_target,
        );
        // Drawn first, behind everything: no depth test or write.
        let sky_pipeline = pipeline(
            "sky pipeline",
            &globals_only_layout,
            "vs_sky",
            "fs_sky",
            &[],
            wgpu::PrimitiveState::default(),
            depth_state(false, Always),
            &opaque_target,
        );
        let grass_pipeline = pipeline(
            "grass pipeline",
            &grass_pipeline_layout,
            "vs_grass",
            "fs_grass",
            &[],
            wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                cull_mode: None,
                ..Default::default()
            },
            depth_state(true, Greater),
            &opaque_target,
        );
        // Water: blended over everything, tested against but not writing depth.
        let water_pipeline = pipeline(
            "water pipeline",
            &globals_only_layout,
            "vs_water",
            "fs_water",
            &[],
            triangles,
            depth_state(false, Greater),
            &blended_target,
        );

        // Depth-only, with the material bound so cut-outs can be discarded.
        let shadow_layout_with_material = layout("shadow layout", &[Some(&globals_layout), Some(&material_layout)]);
        let shadow_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shadow pipeline"),
            layout: Some(&shadow_layout_with_material),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_shadow"),
                compilation_options: Default::default(),
                buffers: &layouts,
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_shadow"),
                compilation_options: Default::default(),
                targets: &[],
            }),
            primitive: triangles,
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                // Slope-scaled bias against shadow acne.
                bias: wgpu::DepthBiasState {
                    constant: 1,
                    slope_scale: 1.0,
                    clamp: 0.0,
                },
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("texture sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 16,
            ..Default::default()
        });

        let pixel = |rgba: [u8; 4], srgb| TextureData {
            width: 1,
            height: 1,
            rgba: rgba.to_vec(),
            srgb,
        };
        let white_texture = upload_texture(&device, &queue, &pixel([255; 4], true));
        let flat_normal_texture = upload_texture(&device, &queue, &pixel([128, 128, 255, 255], false));
        let (depth_view, msaa_view) = create_render_targets(&device, &config);
        let ui_renderer = egui_wgpu::Renderer::new(&device, config.format, Default::default());

        Ok(Self {
            surface,
            device,
            queue,
            config,
            main_pipeline,
            sky_pipeline,
            shadow_pipeline,
            grass_pipeline,
            terrain_pipeline,
            water_pipeline,
            globals_layout,
            grass_layout,
            terrain_layout,
            material_layout,
            grass: None,
            terrain: None,
            globals,
            height_map,
            // No terrain yet: a flat height-0 plane far bigger than any scene.
            terrain_params: [-1000.0, 100_000.0, 200_000.0],
            shadow_bind_group,
            shadow_layer_views,
            sky_occlusion_view,
            sampler,
            white_texture,
            flat_normal_texture,
            depth_view,
            msaa_view,
            batches: Vec::new(),
            screenshot: None,
            world_maps_dirty: true,
            near_cascade_center: None,
            parallax_scale: 1.0,
            ui_renderer,
            plain_material: None,
        })
    }

    /// Saves the next rendered frame as a PNG. `render` returns `true` once
    /// it has been written.
    pub fn request_screenshot(&mut self, path: std::path::PathBuf) {
        self.screenshot = Some(path);
    }

    fn save_frame(&self, texture: &wgpu::Texture, path: &std::path::Path) -> Result<()> {
        anyhow::ensure!(
            self.config.usage.contains(wgpu::TextureUsages::COPY_SRC),
            "this surface does not support reading frames back"
        );
        let (w, h) = (self.config.width, self.config.height);
        let row = (4 * w).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("screenshot"),
            size: (row * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);
        buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        self.device.poll(wgpu::PollType::wait_indefinitely())?;

        let data = buffer.get_mapped_range(..)?;
        let bgra = matches!(
            self.config.format,
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
        );
        let mut pixels = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            let start = (y * row) as usize;
            for px in data[start..start + (w * 4) as usize].chunks_exact(4) {
                if bgra {
                    pixels.extend_from_slice(&[px[2], px[1], px[0], 255]);
                } else {
                    pixels.extend_from_slice(&[px[0], px[1], px[2], 255]);
                }
            }
        }
        image::save_buffer(path, &pixels, w, h, image::ColorType::Rgba8)?;
        Ok(())
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return; // minimized
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        (self.depth_view, self.msaa_view) = create_render_targets(&self.device, &self.config);
    }

    fn identity_instance_buffer(&self) -> wgpu::Buffer {
        self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("identity instance"),
            contents: bytemuck::bytes_of(&InstanceRaw::new(&Instance::IDENTITY)),
            usage: wgpu::BufferUsages::VERTEX,
        })
    }

    /// Uploads a mesh once and draws it at each of `instances` every frame.
    pub fn add_mesh(&mut self, mesh: &MeshData, material: &MaterialData, instances: &[Instance]) {
        if mesh.is_empty() || instances.is_empty() {
            return;
        }
        let device = &self.device;
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("vertices"),
            contents: bytemuck::cast_slice(&mesh.vertices),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("indices"),
            contents: bytemuck::cast_slice(&mesh.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let raw: Vec<InstanceRaw> = instances.iter().map(InstanceRaw::new).collect();
        let instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("instances"),
            contents: bytemuck::cast_slice(&raw),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });

        let uniform = MaterialUniform {
            base_color: material.base_color.to_array(),
            shading: [
                material.alpha_cutoff,
                material.specular,
                material.shininess,
                material.parallax_depth,
            ],
            detail: [
                material.detail_noise,
                material.detail_scale,
                material.macro_variation,
                material.ground_occlusion,
            ],
        };
        let material_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("material"),
            contents: bytemuck::bytes_of(&uniform),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let upload = |t: &Option<Arc<TextureData>>| t.as_ref().map(|t| upload_texture(device, &self.queue, t));
        let base_view = upload(&material.base_color_texture);
        let normal_view = upload(&material.normal_texture);
        let material_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("material"),
            layout: &self.material_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: material_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(base_view.as_ref().unwrap_or(&self.white_texture)),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(
                        normal_view.as_ref().unwrap_or(&self.flat_normal_texture),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });

        if self.plain_material.is_none() && material.alpha_cutoff <= 0.0 {
            self.plain_material = Some(material_bind_group.clone());
        }
        self.batches.push(Batch {
            vertex_buffer,
            index_buffer,
            index_count: mesh.indices.len() as u32,
            instance_buffer,
            instance_count: raw.len() as u32,
            material: material_bind_group,
            visible: true,
            casts_shadow: true,
        });
        self.world_maps_dirty = true;
    }

    /// Sets the terrain: its mesh, the heightmap every shader can query, and
    /// the splat-mapped ground textures.
    pub fn set_terrain(&mut self, terrain: &TerrainData) {
        let device = &self.device;
        self.height_map = upload_heightmap(device, &self.queue, terrain.heights, terrain.res);
        for slot in &mut self.globals {
            slot.bind_group = globals_bind_group(device, &self.globals_layout, &slot.buffer, &self.height_map);
        }
        self.terrain_params = [terrain.water_level, terrain.half_size, terrain.cell];

        let albedo = upload_texture_array(device, &self.queue, &terrain.layers.map(|l| &*l.albedo));
        let normal = upload_texture_array(device, &self.queue, &terrain.layers.map(|l| &*l.normal));
        let splat = upload_texture(device, &self.queue, terrain.splat);
        let inv = terrain.tile_meters.map(|m| 1.0 / m);
        let uniform = TerrainUniform {
            inv_tile: [inv[0], inv[1], inv[2], inv[3]],
            specular: [terrain.specular[0], terrain.specular[1], terrain.specular[2], terrain.specular[3]],
            extra: [inv[4], terrain.specular[4], terrain.macro_variation, 0.0],
        };
        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("terrain"),
            contents: bytemuck::bytes_of(&uniform),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("terrain"),
            layout: &self.terrain_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: wgpu::BindingResource::TextureView(&albedo),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: wgpu::BindingResource::TextureView(&normal),
                },
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: wgpu::BindingResource::TextureView(&splat),
                },
                wgpu::BindGroupEntry {
                    binding: 11,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 12,
                    resource: uniform_buffer.as_entire_binding(),
                },
            ],
        });
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("terrain vertices"),
            contents: bytemuck::cast_slice(&terrain.mesh.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("terrain indices"),
            contents: bytemuck::cast_slice(&terrain.mesh.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        self.terrain = Some(TerrainDraw {
            vertex_buffer,
            index_buffer,
            index_count: terrain.mesh.indices.len() as u32,
            instance_buffer: self.identity_instance_buffer(),
            bind_group,
        });
        self.world_maps_dirty = true;
    }

    /// Enables 3D grass (replacing any previous grass settings).
    pub fn set_grass(&mut self, field: &GrassField) {
        let device = &self.device;
        let side = (2.0 * field.radius / field.spacing).ceil() as u32;
        let uniform = GrassUniform {
            params: [field.half_size, field.spacing, field.radius, side as f32],
            shape: [field.min_height, field.max_height, field.blade_width, field.wind],
            ground: [field.macro_variation, 0.0, 0.0, 0.0],
        };
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("grass"),
            contents: bytemuck::bytes_of(&uniform),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let mask = upload_texture(device, &self.queue, &field.mask);
        let albedo = upload_texture(device, &self.queue, &field.albedo);
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("grass"),
            layout: &self.grass_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(&mask),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(&albedo),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        log::info!("grass: up to {} blades per frame", side * side);
        self.grass = Some(GrassDraw {
            bind_group,
            buffer,
            uniform,
            base_spacing: field.spacing,
            enabled: true,
            instances: side * side,
        });
    }

    /// Adjusts the grass at runtime (from the settings menu): on/off, draw
    /// distance in meters, density multiplier and wind strength.
    pub fn set_grass_options(&mut self, enabled: bool, distance: f32, density: f32, wind: f32) {
        let Some(grass) = &mut self.grass else {
            return;
        };
        let spacing = grass.base_spacing / density.max(0.05).sqrt();
        let side = (2.0 * distance / spacing).ceil() as u32;
        grass.uniform.params[1] = spacing;
        grass.uniform.params[2] = distance;
        grass.uniform.params[3] = side as f32;
        grass.uniform.shape[3] = wind;
        grass.instances = side * side;
        grass.enabled = enabled;
        self.queue
            .write_buffer(&grass.buffer, 0, bytemuck::bytes_of(&grass.uniform));
    }

    /// Turns parallax occlusion mapping (texture extrusion) on or off.
    pub fn set_surface_detail(&mut self, enabled: bool) {
        self.parallax_scale = if enabled { 1.0 } else { 0.0 };
    }

    pub fn set_vsync(&mut self, vsync: bool) {
        let mode = if vsync {
            wgpu::PresentMode::AutoVsync
        } else {
            wgpu::PresentMode::AutoNoVsync
        };
        if self.config.present_mode != mode {
            self.config.present_mode = mode;
            self.surface.configure(&self.device, &self.config);
        }
    }

    /// Re-renders the shadow and sky-occlusion maps next frame (call after
    /// moving the sun).
    pub fn invalidate_shadows(&mut self) {
        self.world_maps_dirty = true;
    }

    /// Adds a single moving object (e.g. the player's body): returns a
    /// handle for set_transform. It doesn't cast shadows (the shadow maps
    /// are only re-rendered when the camera moves a few meters).
    pub fn add_dynamic_mesh(&mut self, mesh: &MeshData, material: &MaterialData) -> usize {
        self.add_mesh(mesh, material, &[Instance::IDENTITY]);
        let index = self.batches.len() - 1;
        self.batches[index].casts_shadow = false;
        index
    }

    /// Replaces a dynamic object's vertices (same count as when it was added),
    /// e.g. an animated character's skinned pose.
    pub fn set_vertices(&mut self, handle: usize, vertices: &[Vertex]) {
        if let Some(batch) = self.batches.get(handle) {
            self.queue
                .write_buffer(&batch.vertex_buffer, 0, bytemuck::cast_slice(vertices));
        }
    }

    /// Moves a dynamic object, and shows or hides it.
    pub fn set_transform(&mut self, handle: usize, transform: Mat4, visible: bool) {
        self.set_transform_tinted(handle, transform, Vec4::ONE, visible);
    }

    /// Like set_transform, also multiplying the object's colour by `tint`.
    pub fn set_transform_tinted(&mut self, handle: usize, transform: Mat4, tint: Vec4, visible: bool) {
        if let Some(batch) = self.batches.get_mut(handle) {
            batch.visible = visible;
            let raw = InstanceRaw::new(&Instance { transform, tint });
            self.queue.write_buffer(&batch.instance_buffer, 0, bytemuck::bytes_of(&raw));
        }
    }

    fn draw_batches(&self, pass: &mut wgpu::RenderPass, with_materials: bool) {
        for batch in &self.batches {
            // The shadow pass is the one drawn without materials.
            if (with_materials && !batch.visible) || (!with_materials && !batch.casts_shadow) {
                continue;
            }
            if with_materials {
                pass.set_bind_group(1, &batch.material, &[]);
            }
            pass.set_vertex_buffer(0, batch.vertex_buffer.slice(..));
            pass.set_vertex_buffer(1, batch.instance_buffer.slice(..));
            pass.set_index_buffer(batch.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..batch.index_count, 0, 0..batch.instance_count);
        }
    }

    /// Shadow-pass drawing: like draw_batches with materials (for cut-out
    /// leaves), skipping objects that don't cast shadows.
    fn draw_shadow_casters(&self, pass: &mut wgpu::RenderPass) {
        for batch in self.batches.iter().filter(|b| b.casts_shadow) {
            pass.set_bind_group(1, &batch.material, &[]);
            pass.set_vertex_buffer(0, batch.vertex_buffer.slice(..));
            pass.set_vertex_buffer(1, batch.instance_buffer.slice(..));
            pass.set_index_buffer(batch.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..batch.index_count, 0, 0..batch.instance_count);
        }
    }

    fn draw_terrain_geometry(&self, pass: &mut wgpu::RenderPass) {
        if let Some(t) = &self.terrain {
            pass.set_vertex_buffer(0, t.vertex_buffer.slice(..));
            pass.set_vertex_buffer(1, t.instance_buffer.slice(..));
            pass.set_index_buffer(t.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..t.index_count, 0, 0..1);
        }
    }

    /// `time` (seconds) animates clouds, grass and water; `ui` is drawn on
    /// top. Returns `true` when a requested screenshot was saved this frame.
    pub fn render(&mut self, camera: &Camera, lighting: &Lighting, time: f32, mut ui: UiFrame) -> Result<bool> {
        // UI textures (e.g. the font atlas) must be uploaded even when this
        // frame ends up being skipped, or they'd be lost.
        for (id, deltas) in &ui.textures_delta.set {
            for delta in deltas {
                self.ui_renderer.update_texture(&self.device, &self.queue, *id, delta);
            }
        }
        let result = self.render_frame(camera, lighting, time, &ui);
        for id in &ui.textures_delta.free {
            self.ui_renderer.free_texture(id);
        }
        // Mark the texture changes as handled (egui checks this).
        ui.textures_delta.clear();
        result
    }

    fn render_frame(&mut self, camera: &Camera, lighting: &Lighting, time: f32, ui: &UiFrame) -> Result<bool> {
        use wgpu::CurrentSurfaceTexture as Current;
        let frame = match self.surface.get_current_texture() {
            Current::Success(frame) => frame,
            Current::Suboptimal(frame) => {
                // Still usable this frame; reconfigure for the next one.
                self.surface.configure(&self.device, &self.config);
                frame
            }
            Current::Outdated => {
                self.surface.configure(&self.device, &self.config);
                return Ok(false);
            }
            Current::Timeout | Current::Occluded => return Ok(false),
            Current::Lost => anyhow::bail!("the window surface was lost"),
            Current::Validation => anyhow::bail!("validation error while acquiring the next frame"),
        };
        let frame_view = frame.texture.create_view(&Default::default());

        // --- light-space matrices -------------------------------------------
        let sun = lighting.sun_direction.normalize();
        let (low, high) = lighting.world_height_range;
        let half = lighting.world_half_size;
        let mid = Vec3::new(0.0, (low + high) * 0.5, 0.0);

        // Near cascade: follows the player in steps, so it is only
        // re-rendered every few meters.
        let snap = |v: f32| (v / NEAR_CASCADE_SNAP).round() * NEAR_CASCADE_SNAP;
        let near_center = Vec3::new(snap(camera.position.x), snap(camera.position.y), snap(camera.position.z));
        let near = sun_ortho(near_center, sun, lighting.shadow_near_radius, 400.0);
        // Far cascade: the whole world, whatever the sun direction.
        let far_radius = half * std::f32::consts::SQRT_2 + 10.0;
        let far = sun_ortho(mid, sun, far_radius, 2.0 * far_radius + 2.0 * (high - low) + 100.0);
        // Sky occlusion: straight down over the whole world.
        let top = high + 20.0;
        let sky_view = camera::rh::view::look_to_mat4(Vec3::new(0.0, top, 0.0), Vec3::NEG_Y, Vec3::NEG_Z);
        let sky_view_proj =
            camera::rh::proj::directx::orthographic(-half, half, -half, half, 0.1, top - low + 10.0) * sky_view;

        let view_proj = camera.view_projection();
        let mut globals = GlobalsUniform {
            view_proj: view_proj.to_cols_array_2d(),
            inv_view_proj: view_proj.inverse().to_cols_array_2d(),
            light_view_proj: near.to_cols_array_2d(),
            far_light_view_proj: far.to_cols_array_2d(),
            sky_view_proj: sky_view_proj.to_cols_array_2d(),
            camera_pos: camera.position.extend(time).to_array(),
            sun_dir: sun.extend(0.0).to_array(),
            sun_color: lighting.sun_color.extend(1.0).to_array(),
            sky_zenith: lighting.sky_zenith.extend(1.0).to_array(),
            sky_horizon: lighting.sky_horizon.extend(1.0).to_array(),
            sky_ambient: lighting.sky_ambient.extend(1.0).to_array(),
            ground_ambient: lighting.ground_ambient.extend(1.0).to_array(),
            params: [
                lighting.fog_start,
                lighting.fog_end,
                lighting.exposure,
                1.0 / SHADOW_MAP_SIZE as f32,
            ],
            extra: [
                self.parallax_scale,
                self.terrain_params[0],
                self.terrain_params[1],
                self.terrain_params[2],
            ],
        };
        self.queue
            .write_buffer(&self.globals[0].buffer, 0, bytemuck::bytes_of(&globals));
        globals.light_view_proj = far.to_cols_array_2d();
        self.queue
            .write_buffer(&self.globals[1].buffer, 0, bytemuck::bytes_of(&globals));
        globals.light_view_proj = sky_view_proj.to_cols_array_2d();
        self.queue
            .write_buffer(&self.globals[2].buffer, 0, bytemuck::bytes_of(&globals));

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });

        // 1. Depth-only passes, only for what changed.
        let mut depth_passes: Vec<(&wgpu::TextureView, &wgpu::BindGroup)> = Vec::new();
        if self.world_maps_dirty || self.near_cascade_center != Some(near_center) {
            depth_passes.push((&self.shadow_layer_views[0], &self.globals[0].bind_group));
        }
        if self.world_maps_dirty {
            depth_passes.push((&self.shadow_layer_views[1], &self.globals[1].bind_group));
            depth_passes.push((&self.sky_occlusion_view, &self.globals[2].bind_group));
        }
        for (target, globals) in depth_passes {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("depth-only pass"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: target,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_pipeline(&self.shadow_pipeline);
            pass.set_bind_group(0, globals, &[]);
            if let Some(plain) = &self.plain_material {
                pass.set_bind_group(1, plain, &[]);
            }
            self.draw_terrain_geometry(&mut pass);
            self.draw_shadow_casters(&mut pass);
        }
        self.world_maps_dirty = false;
        self.near_cascade_center = Some(near_center);

        // 2. Sky, terrain, meshes, grass, water.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.msaa_view,
                    depth_slice: None,
                    resolve_target: Some(&frame_view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        // Reverse-Z: 0 is infinitely far.
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });

            pass.set_bind_group(0, &self.globals[0].bind_group, &[]);
            pass.set_pipeline(&self.sky_pipeline);
            pass.draw(0..3, 0..1);

            if let Some(terrain) = &self.terrain {
                pass.set_pipeline(&self.terrain_pipeline);
                pass.set_bind_group(1, &terrain.bind_group, &[]);
                pass.set_bind_group(2, &self.shadow_bind_group, &[]);
                self.draw_terrain_geometry(&mut pass);
            }

            pass.set_pipeline(&self.main_pipeline);
            pass.set_bind_group(2, &self.shadow_bind_group, &[]);
            self.draw_batches(&mut pass, true);

            if let Some(grass) = self.grass.as_ref().filter(|g| g.enabled) {
                pass.set_pipeline(&self.grass_pipeline);
                pass.set_bind_group(1, &grass.bind_group, &[]);
                // Group 1 changed layout, so re-bind what follows it.
                pass.set_bind_group(2, &self.shadow_bind_group, &[]);
                pass.draw(0..7, 0..grass.instances);
            }

            if self.terrain.is_some() {
                pass.set_pipeline(&self.water_pipeline);
                pass.draw(0..6, 0..1);
            }
        }

        // 3. UI on top (not multisampled; egui anti-aliases its own shapes).
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [self.config.width, self.config.height],
            pixels_per_point: ui.pixels_per_point,
        };
        let ui_commands =
            self.ui_renderer
                .update_buffers(&self.device, &self.queue, &mut encoder, &ui.primitives, &screen);
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("ui pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &frame_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                })
                .forget_lifetime();
            self.ui_renderer.render(&mut pass, &ui.primitives, &screen);
        }
        self.queue.submit(ui_commands.into_iter().chain([encoder.finish()]));

        let mut saved = false;
        if let Some(path) = self.screenshot.take() {
            self.save_frame(&frame.texture, &path)
                .with_context(|| format!("saving screenshot to {}", path.display()))?;
            log::info!("saved screenshot to {}", path.display());
            saved = true;
        }
        self.queue.present(frame);
        Ok(saved)
    }
}

/// Orthographic view-projection looking along the sunlight, centred on
/// `center`, `radius` wide and `depth` deep.
fn sun_ortho(center: Vec3, sun: Vec3, radius: f32, depth: f32) -> Mat4 {
    let up = if sun.y.abs() > 0.99 { Vec3::Z } else { Vec3::Y };
    let view = camera::rh::view::look_to_mat4(center - sun * (depth * 0.5), sun, up);
    camera::rh::proj::directx::orthographic(-radius, radius, -radius, radius, 0.1, depth) * view
}

fn globals_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    buffer: &wgpu::Buffer,
    height_map: &wgpu::TextureView,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("globals"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(height_map),
            },
        ],
    })
}

fn create_render_targets(
    device: &wgpu::Device,
    config: &wgpu::SurfaceConfiguration,
) -> (wgpu::TextureView, wgpu::TextureView) {
    let size = wgpu::Extent3d {
        width: config.width,
        height: config.height,
        depth_or_array_layers: 1,
    };
    let make = |label, format| {
        device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size,
                mip_level_count: 1,
                sample_count: SAMPLE_COUNT,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&Default::default())
    };
    (make("depth", DEPTH_FORMAT), make("msaa color", config.format))
}

fn upload_heightmap(device: &wgpu::Device, queue: &wgpu::Queue, heights: &[f32], res: u32) -> wgpu::TextureView {
    let size = wgpu::Extent3d {
        width: res,
        height: res,
        depth_or_array_layers: 1,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("height map"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        bytemuck::cast_slice(heights),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4 * res),
            rows_per_image: Some(res),
        },
        size,
    );
    texture.create_view(&Default::default())
}

fn texture_format(srgb: bool) -> wgpu::TextureFormat {
    if srgb {
        wgpu::TextureFormat::Rgba8UnormSrgb
    } else {
        wgpu::TextureFormat::Rgba8Unorm
    }
}

/// Uploads a texture with a full mip chain (box-filtered on the CPU).
fn upload_texture(device: &wgpu::Device, queue: &wgpu::Queue, data: &TextureData) -> wgpu::TextureView {
    upload_layers(device, queue, &[data], wgpu::TextureViewDimension::D2)
}

/// Uploads same-sized textures as the layers of one texture array.
fn upload_texture_array(device: &wgpu::Device, queue: &wgpu::Queue, layers: &[&TextureData]) -> wgpu::TextureView {
    upload_layers(device, queue, layers, wgpu::TextureViewDimension::D2Array)
}

fn upload_layers(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layers: &[&TextureData],
    dimension: wgpu::TextureViewDimension,
) -> wgpu::TextureView {
    let first = layers[0];
    let mip_count = build_mip_chain(first).len() as u32;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("material texture"),
        size: wgpu::Extent3d {
            width: first.width,
            height: first.height,
            depth_or_array_layers: layers.len() as u32,
        },
        mip_level_count: mip_count,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: texture_format(first.srgb),
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (layer, data) in layers.iter().enumerate() {
        assert!(data.width == first.width && data.height == first.height, "texture array layers must match in size");
        for (level, (w, h, pixels)) in build_mip_chain(data).iter().enumerate() {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: level as u32,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: 0,
                        z: layer as u32,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * w),
                    rows_per_image: Some(*h),
                },
                wgpu::Extent3d {
                    width: *w,
                    height: *h,
                    depth_or_array_layers: 1,
                },
            );
        }
    }
    texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(dimension),
        ..Default::default()
    })
}

fn build_mip_chain(data: &TextureData) -> Vec<(u32, u32, Vec<u8>)> {
    let mut levels = vec![(data.width, data.height, data.rgba.clone())];
    loop {
        let (w, h, prev) = levels.last().unwrap();
        let (w, h) = (*w, *h);
        if w == 1 && h == 1 {
            break;
        }
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let mut next = vec![0u8; (nw * nh * 4) as usize];
        for y in 0..nh {
            for x in 0..nw {
                for c in 0..4 {
                    let mut sum = 0u32;
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let sx = (2 * x + dx).min(w - 1);
                        let sy = (2 * y + dy).min(h - 1);
                        sum += prev[((sy * w + sx) * 4 + c) as usize] as u32;
                    }
                    next[((y * nw + x) * 4 + c) as usize] = (sum / 4) as u8;
                }
            }
        }
        levels.push((nw, nh, next));
    }
    levels
}
