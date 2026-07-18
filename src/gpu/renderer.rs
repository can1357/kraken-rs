use std::{collections::HashMap, mem, ops::Range};

use anyhow::{Context, Result};
use bytemuck::{Pod, Zeroable};
use glyphon::{
    Attrs, Buffer as TextBuffer, Cache, Color as GlyphColor, ColorMode, Family, FontSystem,
    Metrics, Resolution, Shaping, SwashCache, TextArea, TextAtlas, TextBounds, TextRenderer,
    Viewport, Weight,
};
use num_traits::ToPrimitive;
use wgpu::util::DeviceExt;

use crate::ui::{
    Color,
    scene::{FontFace, LAYER_COUNT, Scene, TextSpec},
};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    size: [f32; 2],
    padding: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct QuadVertex {
    corner: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct RectInstance {
    rect: [f32; 4],
    clip: [f32; 4],
    fill: [f32; 4],
    border: [f32; 4],
    params: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuMeshVertex {
    position: [f32; 2],
    color: [f32; 4],
    clip: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ImageInstance {
    rect: [f32; 4],
    clip: [f32; 4],
    uv: [f32; 4],
}

const MAX_RETAINED_TEXT_BUFFERS: usize = 4_096;
const RETAINED_TEXT_GENERATIONS: u64 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum WrappingMode {
    None,
    WordOrGlyph,
}

impl WrappingMode {
    fn glyphon(self) -> glyphon::Wrap {
        match self {
            Self::None => glyphon::Wrap::None,
            Self::WordOrGlyph => glyphon::Wrap::WordOrGlyph,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct ShapingProperties {
    face: FontFace,
    size: u32,
    line_height: u32,
    width: u32,
    height: u32,
    wrapping: WrappingMode,
}

impl ShapingProperties {
    fn for_spec(spec: &TextSpec) -> Self {
        let wrapping = if spec.bounds.height < spec.line_height * 1.9 {
            WrappingMode::None
        } else {
            WrappingMode::WordOrGlyph
        };
        Self {
            face: spec.face,
            size: spec.size.to_bits(),
            line_height: spec.line_height.to_bits(),
            width: spec.bounds.width.to_bits(),
            height: spec.bounds.height.to_bits(),
            wrapping,
        }
    }
}

struct CachedTextBuffer {
    buffer: TextBuffer,
    generation: u64,
    last_used: u64,
}

#[derive(Default)]
struct TextBufferCache {
    buffers: HashMap<ShapingProperties, HashMap<String, CachedTextBuffer>>,
    generation: u64,
    last_used: u64,
    len: usize,
}

impl TextBufferCache {
    fn begin_frame(&mut self) {
        self.generation = self.generation.saturating_add(1);
    }

    fn retain(&mut self, font_system: &mut FontSystem, spec: &TextSpec) {
        self.last_used = self.last_used.saturating_add(1);
        let last_used = self.last_used;
        let properties = ShapingProperties::for_spec(spec);
        let buffers = self.buffers.entry(properties).or_default();
        if let Some(cached) = buffers.get_mut(spec.text.as_str()) {
            cached.generation = self.generation;
            cached.last_used = last_used;
            return;
        }

        let mut buffer = TextBuffer::new_empty(Metrics::new(spec.size, spec.line_height));
        buffer.set_size(Some(spec.bounds.width), Some(spec.bounds.height));
        buffer.set_wrap(properties.wrapping.glyphon());
        set_shaping_text(&mut buffer, spec);
        buffer.shape_until_scroll(font_system, false);
        buffers.insert(
            spec.text.clone(),
            CachedTextBuffer {
                buffer,
                generation: self.generation,
                last_used,
            },
        );
        self.len += 1;
    }

    fn get(&self, spec: &TextSpec) -> &TextBuffer {
        let properties = ShapingProperties::for_spec(spec);
        &self
            .buffers
            .get(&properties)
            .and_then(|buffers| buffers.get(spec.text.as_str()))
            .expect("current-frame text buffer retained")
            .buffer
    }

    fn trim(&mut self) {
        let oldest_generation = self.generation.saturating_sub(RETAINED_TEXT_GENERATIONS);
        let mut removed = 0;
        self.buffers.retain(|_, buffers| {
            buffers.retain(|_, cached| {
                let retain = cached.generation >= oldest_generation;
                removed += usize::from(!retain);
                retain
            });
            !buffers.is_empty()
        });
        self.len = self.len.saturating_sub(removed);
        if self.len <= MAX_RETAINED_TEXT_BUFFERS {
            return;
        }

        let remove_count = self.len - MAX_RETAINED_TEXT_BUFFERS;
        let mut uses = Vec::with_capacity(self.len);
        uses.extend(
            self.buffers
                .values()
                .flat_map(|buffers| buffers.values().map(|cached| cached.last_used)),
        );
        uses.sort_unstable();
        let cutoff = uses[remove_count - 1];
        let mut remove_at_cutoff = uses[..remove_count]
            .iter()
            .filter(|last_used| **last_used == cutoff)
            .count();
        removed = 0;
        self.buffers.retain(|_, buffers| {
            buffers.retain(|_, cached| {
                let remove = cached.last_used < cutoff
                    || (cached.last_used == cutoff && remove_at_cutoff > 0 && {
                        remove_at_cutoff -= 1;
                        true
                    });
                removed += usize::from(remove);
                !remove
            });
            !buffers.is_empty()
        });
        self.len = self.len.saturating_sub(removed);
    }
}

struct DynamicBuffer {
    buffer: wgpu::Buffer,
    capacity: usize,
    usage: wgpu::BufferUsages,
    label: &'static str,
}

impl DynamicBuffer {
    fn new(device: &wgpu::Device, usage: wgpu::BufferUsages, label: &'static str) -> Self {
        let capacity = 256;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: capacity,
            usage: usage | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            buffer,
            capacity: capacity.to_usize().unwrap_or(256),
            usage,
            label,
        }
    }

    fn upload<T: Pod>(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, values: &[T]) {
        let required = mem::size_of_val(values).max(mem::size_of::<T>());
        if required > self.capacity {
            self.capacity = required.next_power_of_two();
            self.buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(self.label),
                size: self.capacity.to_u64().unwrap_or(u64::MAX),
                usage: self.usage | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        if !values.is_empty() {
            queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(values));
        }
    }
}

struct FrostTargets {
    width: u32,
    height: u32,
    base_view: wgpu::TextureView,
    blur_a_view: wgpu::TextureView,
    blur_b_view: wgpu::TextureView,
    base_bind_group: wgpu::BindGroup,
    blur_a_bind_group: wgpu::BindGroup,
    blur_b_bind_group: wgpu::BindGroup,
}

/// Owns the custom chrome/graph pipelines and glyphon atlas for one GPU device.
pub(crate) struct Renderer {
    globals: wgpu::Buffer,
    globals_bind_group: wgpu::BindGroup,
    quad: wgpu::Buffer,
    rectangle_instances: DynamicBuffer,
    frost_instances: DynamicBuffer,
    mesh_vertices: DynamicBuffer,
    format: wgpu::TextureFormat,
    frost_layout: wgpu::BindGroupLayout,
    frost_sampler: wgpu::Sampler,
    copy_pipeline: wgpu::RenderPipeline,
    downsample_pipeline: wgpu::RenderPipeline,
    blur_horizontal_pipeline: wgpu::RenderPipeline,
    blur_vertical_pipeline: wgpu::RenderPipeline,
    frost_pipeline: wgpu::RenderPipeline,
    frost_targets: Option<FrostTargets>,
    rectangle_pipeline: wgpu::RenderPipeline,
    mesh_pipeline: wgpu::RenderPipeline,
    image_instances: DynamicBuffer,
    image_pipeline: wgpu::RenderPipeline,
    avatar_texture: wgpu::Texture,
    avatar_bind_group: wgpu::BindGroup,
    avatar_slots: HashMap<String, u32>,
    next_avatar_slot: u32,
    font_system: FontSystem,
    swash_cache: SwashCache,
    viewport: Viewport,
    text_atlas: TextAtlas,
    glyphs: [TextRenderer; LAYER_COUNT],
    text_buffers: TextBufferCache,
}

impl Renderer {
    /// Creates pipelines compatible with a surface or offscreen texture format.
    pub(crate) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
    ) -> Self {
        let globals = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ui globals"),
            contents: bytemuck::bytes_of(&Globals {
                size: [1.0, 1.0],
                padding: [0.0, 0.0],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ui globals layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let globals_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ui globals bind group"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ui pipeline layout"),
            bind_group_layouts: &[Some(&globals_layout)],
            immediate_size: 0,
        });

        let frost_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("frost composite shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("frost.wgsl").into()),
        });
        let frost_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("frost source layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let frost_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("frost linear sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..wgpu::SamplerDescriptor::default()
        });
        let frost_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("frost pipeline layout"),
                bind_group_layouts: &[Some(&globals_layout), Some(&frost_layout)],
                immediate_size: 0,
            });
        let fullscreen_pipeline = |label: &'static str, entry_point: &'static str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&frost_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &frost_shader,
                    entry_point: Some("vs_fullscreen"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &frost_shader,
                    entry_point: Some(entry_point),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(color_target(format))],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let copy_pipeline = fullscreen_pipeline("frost backdrop copy", "fs_copy");
        let downsample_pipeline = fullscreen_pipeline("frost backdrop downsample", "fs_downsample");
        let blur_horizontal_pipeline =
            fullscreen_pipeline("frost backdrop horizontal blur", "fs_blur_horizontal");
        let blur_vertical_pipeline =
            fullscreen_pipeline("frost backdrop vertical blur", "fs_blur_vertical");
        let frost_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("frosted popup pipeline"),
            layout: Some(&frost_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &frost_shader,
                entry_point: Some("vs_frost"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[
                    Some(wgpu::VertexBufferLayout {
                        array_stride: mem::size_of::<QuadVertex>().to_u64().unwrap_or(0),
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x2],
                    }),
                    Some(wgpu::VertexBufferLayout {
                        array_stride: mem::size_of::<RectInstance>().to_u64().unwrap_or(0),
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &wgpu::vertex_attr_array![
                            1 => Float32x4,
                            2 => Float32x4,
                            3 => Float32x4,
                            4 => Float32x4,
                            5 => Float32x4
                        ],
                    }),
                ],
            },
            fragment: Some(wgpu::FragmentState {
                module: &frost_shader,
                entry_point: Some("fs_frost"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(color_target(format))],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let rectangle_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rounded rectangle shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("rounded.wgsl").into()),
        });
        let mesh_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("mesh shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("mesh.wgsl").into()),
        });
        let rectangle_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("rounded rectangle pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &rectangle_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[
                    Some(wgpu::VertexBufferLayout {
                        array_stride: mem::size_of::<QuadVertex>().to_u64().unwrap_or(0),
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x2],
                    }),
                    Some(wgpu::VertexBufferLayout {
                        array_stride: mem::size_of::<RectInstance>().to_u64().unwrap_or(0),
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &wgpu::vertex_attr_array![
                            1 => Float32x4,
                            2 => Float32x4,
                            3 => Float32x4,
                            4 => Float32x4,
                            5 => Float32x4
                        ],
                    }),
                ],
            },
            fragment: Some(wgpu::FragmentState {
                module: &rectangle_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(color_target(format))],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let mesh_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("line and curve mesh pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &mesh_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: mem::size_of::<GpuMeshVertex>().to_u64().unwrap_or(0),
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4, 2 => Float32x4],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &mesh_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(color_target(format))],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let avatar_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("avatar atlas shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("avatar.wgsl").into()),
        });
        let avatar_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("avatar atlas layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let avatar_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("avatar atlas pipeline layout"),
                bind_group_layouts: &[Some(&globals_layout), Some(&avatar_layout)],
                immediate_size: 0,
            });
        let image_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("avatar atlas pipeline"),
            layout: Some(&avatar_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &avatar_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[
                    Some(wgpu::VertexBufferLayout {
                        array_stride: mem::size_of::<QuadVertex>().to_u64().unwrap_or(0),
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x2],
                    }),
                    Some(wgpu::VertexBufferLayout {
                        array_stride: mem::size_of::<ImageInstance>().to_u64().unwrap_or(0),
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &wgpu::vertex_attr_array![1 => Float32x4, 2 => Float32x4, 3 => Float32x4],
                    }),
                ],
            },
            fragment: Some(wgpu::FragmentState {
                module: &avatar_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(color_target(format))],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let avatar_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("avatar atlas"),
            size: wgpu::Extent3d {
                width: 1024,
                height: 1024,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let avatar_view = avatar_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let avatar_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("avatar atlas sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..wgpu::SamplerDescriptor::default()
        });
        let avatar_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("avatar atlas bind group"),
            layout: &avatar_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&avatar_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&avatar_sampler),
                },
            ],
        });
        let quad_vertices = [
            QuadVertex { corner: [0.0, 0.0] },
            QuadVertex { corner: [1.0, 0.0] },
            QuadVertex { corner: [0.0, 1.0] },
            QuadVertex { corner: [0.0, 1.0] },
            QuadVertex { corner: [1.0, 0.0] },
            QuadVertex { corner: [1.0, 1.0] },
        ];
        let quad = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("rounded rectangle quad"),
            contents: bytemuck::cast_slice(&quad_vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let cache = Cache::new(device);
        let viewport = Viewport::new(device, &cache);
        // `Web` blends glyph coverage in gamma space like native/browser text
        // stacks; `Accurate` (linear) thins dark-on-light strokes and bloats
        // light-on-dark ones on sRGB targets.
        let mut text_atlas =
            TextAtlas::with_color_mode(device, queue, &cache, format, ColorMode::Web);
        let glyphs = std::array::from_fn(|_| {
            TextRenderer::new(
                &mut text_atlas,
                device,
                wgpu::MultisampleState::default(),
                None,
            )
        });

        Self {
            globals,
            globals_bind_group,
            quad,
            rectangle_instances: DynamicBuffer::new(
                device,
                wgpu::BufferUsages::VERTEX,
                "rounded rectangle instances",
            ),
            mesh_vertices: DynamicBuffer::new(
                device,
                wgpu::BufferUsages::VERTEX,
                "line and curve vertices",
            ),
            rectangle_pipeline,
            mesh_pipeline,
            frost_instances: DynamicBuffer::new(
                device,
                wgpu::BufferUsages::VERTEX,
                "frosted popup instances",
            ),
            format,
            frost_layout,
            frost_sampler,
            copy_pipeline,
            downsample_pipeline,
            blur_horizontal_pipeline,
            blur_vertical_pipeline,
            frost_pipeline,
            frost_targets: None,
            image_instances: DynamicBuffer::new(
                device,
                wgpu::BufferUsages::VERTEX,
                "avatar instances",
            ),
            image_pipeline,
            avatar_texture,
            avatar_bind_group,
            avatar_slots: HashMap::new(),
            next_avatar_slot: 0,
            font_system: brand_font_system(),
            swash_cache: SwashCache::new(),
            viewport,
            text_atlas,
            glyphs,
            text_buffers: TextBufferCache::default(),
        }
    }

    /// Encodes one complete immediate-mode scene into an existing target view.
    pub(crate) fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        scene: &Scene,
        clear: Color,
    ) -> Result<()> {
        let width = scene.width.round().to_u32().unwrap_or(1).max(1);
        let height = scene.height.round().to_u32().unwrap_or(1).max(1);
        self.write_globals(queue, width, height);
        self.viewport.update(queue, Resolution { width, height });

        let (rectangles, rectangle_ranges, frost_rectangles, frost_ranges) =
            collect_rectangles(scene);
        let has_frost = !frost_rectangles.is_empty();
        self.rectangle_instances.upload(device, queue, &rectangles);
        self.frost_instances
            .upload(device, queue, &frost_rectangles);
        let (mesh, mesh_ranges) = collect_mesh(scene);
        self.mesh_vertices.upload(device, queue, &mesh);
        let (images, image_ranges) = self.collect_images(queue, scene);
        self.image_instances.upload(device, queue, &images);
        self.prepare_text(device, queue, scene)?;

        if !has_frost {
            let mut pass = frame_pass(
                encoder,
                target,
                "kraken frame",
                wgpu::LoadOp::Clear(to_wgpu_color(clear)),
            );
            self.draw_layers(
                &mut pass,
                0..LAYER_COUNT,
                &rectangle_ranges,
                &mesh_ranges,
                &image_ranges,
            )?;
            drop(pass);
            self.text_atlas.trim();
            return Ok(());
        }

        self.ensure_frost_targets(device, width, height);
        let (
            base_view,
            blur_a_view,
            blur_b_view,
            base_bind_group,
            blur_a_bind_group,
            blur_b_bind_group,
        ) = {
            let targets = self
                .frost_targets
                .as_ref()
                .expect("frost targets initialized");
            (
                targets.base_view.clone(),
                targets.blur_a_view.clone(),
                targets.blur_b_view.clone(),
                targets.base_bind_group.clone(),
                targets.blur_a_bind_group.clone(),
                targets.blur_b_bind_group.clone(),
            )
        };
        {
            let mut pass = frame_pass(
                encoder,
                &base_view,
                "frost backdrop",
                wgpu::LoadOp::Clear(to_wgpu_color(clear)),
            );
            // Layers 0..4 (including drop shadows on layer 3) render into the
            // backdrop so glass shows a blurred copy of everything beneath it.
            self.draw_layers(
                &mut pass,
                0..4,
                &rectangle_ranges,
                &mesh_ranges,
                &image_ranges,
            )?;
        }
        let blur_width = (width + 3) / 4;
        let blur_height = (height + 3) / 4;
        self.write_globals(queue, blur_width, blur_height);
        self.fullscreen_pass(
            encoder,
            &blur_a_view,
            "frost downsample",
            &self.downsample_pipeline,
            &base_bind_group,
        );
        self.fullscreen_pass(
            encoder,
            &blur_b_view,
            "frost horizontal blur",
            &self.blur_horizontal_pipeline,
            &blur_a_bind_group,
        );
        self.fullscreen_pass(
            encoder,
            &blur_a_view,
            "frost vertical blur",
            &self.blur_vertical_pipeline,
            &blur_b_bind_group,
        );
        self.write_globals(queue, width, height);
        {
            let mut pass = frame_pass(
                encoder,
                target,
                "frost composite",
                wgpu::LoadOp::Clear(to_wgpu_color(clear)),
            );
            pass.set_pipeline(&self.copy_pipeline);
            pass.set_bind_group(0, &self.globals_bind_group, &[]);
            pass.set_bind_group(1, &base_bind_group, &[]);
            pass.draw(0..3, 0..1);
            pass.set_pipeline(&self.frost_pipeline);
            pass.set_bind_group(0, &self.globals_bind_group, &[]);
            pass.set_bind_group(1, &blur_a_bind_group, &[]);
            pass.set_vertex_buffer(0, self.quad.slice(..));
            pass.set_vertex_buffer(1, self.frost_instances.buffer.slice(..));
            for range in &frost_ranges {
                if !range.is_empty() {
                    pass.draw(0..6, range.clone());
                }
            }
            self.draw_layers(
                &mut pass,
                4..LAYER_COUNT,
                &rectangle_ranges,
                &mesh_ranges,
                &image_ranges,
            )?;
        }
        self.text_atlas.trim();
        Ok(())
    }

    fn fullscreen_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        label: &'static str,
        pipeline: &wgpu::RenderPipeline,
        source: &wgpu::BindGroup,
    ) {
        let mut pass = frame_pass(
            encoder,
            target,
            label,
            wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
        );
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &self.globals_bind_group, &[]);
        pass.set_bind_group(1, source, &[]);
        pass.draw(0..3, 0..1);
    }

    fn ensure_frost_targets(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        if self
            .frost_targets
            .as_ref()
            .is_some_and(|targets| targets.width == width && targets.height == height)
        {
            return;
        }
        let create_texture = |label, width, height| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        };
        let base = create_texture("frost backdrop texture", width, height);
        let blur_a = create_texture("frost blur ping texture", (width + 3) / 4, (height + 3) / 4);
        let blur_b = create_texture("frost blur pong texture", (width + 3) / 4, (height + 3) / 4);
        let base_view = base.create_view(&wgpu::TextureViewDescriptor::default());
        let blur_a_view = blur_a.create_view(&wgpu::TextureViewDescriptor::default());
        let blur_b_view = blur_b.create_view(&wgpu::TextureViewDescriptor::default());
        let make_bind_group = |label, view: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &self.frost_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.frost_sampler),
                    },
                ],
            })
        };
        self.frost_targets = Some(FrostTargets {
            width,
            height,
            base_view: base_view.clone(),
            blur_a_view: blur_a_view.clone(),
            blur_b_view: blur_b_view.clone(),
            base_bind_group: make_bind_group("frost backdrop bind group", &base_view),
            blur_a_bind_group: make_bind_group("frost ping bind group", &blur_a_view),
            blur_b_bind_group: make_bind_group("frost pong bind group", &blur_b_view),
        });
    }

    fn write_globals(&self, queue: &wgpu::Queue, width: u32, height: u32) {
        queue.write_buffer(
            &self.globals,
            0,
            bytemuck::bytes_of(&Globals {
                size: [width as f32, height as f32],
                padding: [0.0, 0.0],
            }),
        );
    }

    fn draw_layers(
        &mut self,
        pass: &mut wgpu::RenderPass<'_>,
        layers: Range<usize>,
        rectangle_ranges: &[Range<u32>; LAYER_COUNT],
        mesh_ranges: &[Range<u32>; LAYER_COUNT],
        image_ranges: &[Range<u32>; LAYER_COUNT],
    ) -> Result<()> {
        for layer in layers {
            pass.set_bind_group(0, &self.globals_bind_group, &[]);
            let rectangle_range = rectangle_ranges[layer].clone();
            if !rectangle_range.is_empty() {
                pass.set_pipeline(&self.rectangle_pipeline);
                pass.set_vertex_buffer(0, self.quad.slice(..));
                pass.set_vertex_buffer(1, self.rectangle_instances.buffer.slice(..));
                pass.draw(0..6, rectangle_range);
            }
            let mesh_range = mesh_ranges[layer].clone();
            if !mesh_range.is_empty() {
                pass.set_pipeline(&self.mesh_pipeline);
                pass.set_vertex_buffer(0, self.mesh_vertices.buffer.slice(..));
                pass.draw(mesh_range, 0..1);
            }
            let image_range = image_ranges[layer].clone();
            if !image_range.is_empty() {
                pass.set_pipeline(&self.image_pipeline);
                pass.set_bind_group(1, &self.avatar_bind_group, &[]);
                pass.set_vertex_buffer(0, self.quad.slice(..));
                pass.set_vertex_buffer(1, self.image_instances.buffer.slice(..));
                pass.draw(0..6, image_range);
            }
            self.glyphs[layer]
                .render(&self.text_atlas, &self.viewport, pass)
                .context("render glyph atlas")?;
        }
        Ok(())
    }

    fn collect_images(
        &mut self,
        queue: &wgpu::Queue,
        scene: &Scene,
    ) -> (Vec<ImageInstance>, [Range<u32>; LAYER_COUNT]) {
        const ATLAS: u32 = 1024;
        const CELL: u32 = 64;
        let total = scene.layers.iter().map(|layer| layer.images.len()).sum();
        let mut instances = Vec::with_capacity(total);
        let mut ranges = std::array::from_fn(|_| 0..0);
        for (layer_index, layer) in scene.layers.iter().enumerate() {
            let start = instances.len().to_u32().unwrap_or(u32::MAX);
            for image in &layer.images {
                let slot = if let Some(slot) = self.avatar_slots.get(&image.key) {
                    *slot
                } else {
                    if self.next_avatar_slot == 256 {
                        self.avatar_slots.clear();
                        self.next_avatar_slot = 0;
                    }
                    let slot = self.next_avatar_slot;
                    self.next_avatar_slot += 1;
                    self.avatar_slots.insert(image.key.clone(), slot);
                    slot
                };
                let x = (slot % 16) * CELL;
                let y = (slot / 16) * CELL;
                let pixels = crate::graph::avatars::pixels(&image.key);
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &self.avatar_texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d { x, y, z: 0 },
                        aspect: wgpu::TextureAspect::All,
                    },
                    &pixels,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(CELL * 4),
                        rows_per_image: Some(CELL),
                    },
                    wgpu::Extent3d {
                        width: CELL,
                        height: CELL,
                        depth_or_array_layers: 1,
                    },
                );
                instances.push(ImageInstance {
                    rect: [
                        image.rect.x,
                        image.rect.y,
                        image.rect.width,
                        image.rect.height,
                    ],
                    clip: [
                        image.clip.x,
                        image.clip.y,
                        image.clip.right(),
                        image.clip.bottom(),
                    ],
                    uv: [
                        x as f32 / ATLAS as f32,
                        y as f32 / ATLAS as f32,
                        CELL as f32 / ATLAS as f32,
                        CELL as f32 / ATLAS as f32,
                    ],
                });
            }
            ranges[layer_index] = start..instances.len().to_u32().unwrap_or(u32::MAX);
        }
        (instances, ranges)
    }

    fn prepare_text(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &Scene,
    ) -> Result<()> {
        self.text_buffers.begin_frame();
        for layer in &scene.layers {
            for spec in &layer.text {
                self.text_buffers.retain(&mut self.font_system, spec);
            }
        }

        for (layer_index, layer) in scene.layers.iter().enumerate() {
            let areas = layer.text.iter().map(|spec| TextArea {
                buffer: self.text_buffers.get(spec),
                left: spec.origin[0],
                top: spec.origin[1],
                scale: 1.0,
                bounds: TextBounds {
                    left: spec.bounds.x.floor().to_i32().unwrap_or(i32::MIN),
                    top: spec.bounds.y.floor().to_i32().unwrap_or(i32::MIN),
                    right: spec.bounds.right().ceil().to_i32().unwrap_or(i32::MAX),
                    bottom: spec.bounds.bottom().ceil().to_i32().unwrap_or(i32::MAX),
                },
                default_color: to_glyph_color(spec.color),
                custom_glyphs: &[],
            });
            self.glyphs[layer_index]
                .prepare(
                    device,
                    queue,
                    &mut self.font_system,
                    &mut self.text_atlas,
                    &self.viewport,
                    areas,
                    &mut self.swash_cache,
                )
                .context("prepare glyph atlas")?;
        }
        self.text_buffers.trim();
        self.font_system.shape_run_cache.trim(2);
        Ok(())
    }
}

fn set_shaping_text(buffer: &mut TextBuffer, spec: &TextSpec) {
    let default_attrs = Attrs::new()
        .family(font_family(spec.face))
        .weight(face_weight(spec.face));
    let is_sans = matches!(
        spec.face,
        FontFace::Sans | FontFace::SansMedium | FontFace::SansBold
    );
    if !is_sans {
        buffer.set_text(&spec.text, &default_attrs, Shaping::Advanced, None);
        return;
    }

    let mut spans = Vec::new();
    let mut start = 0;
    let mut span_is_icon = None;
    for (index, character) in spec.text.char_indices() {
        let is_icon = crate::ui::icons::is_private_use(character);
        if span_is_icon.is_some_and(|current| current != is_icon) {
            spans.push((start..index, span_is_icon.unwrap_or(false)));
            start = index;
        }
        span_is_icon = Some(is_icon);
    }
    let Some(last_is_icon) = span_is_icon else {
        buffer.set_text(&spec.text, &default_attrs, Shaping::Advanced, None);
        return;
    };
    spans.push((start..spec.text.len(), last_is_icon));
    if !spans.iter().any(|(_, is_icon)| *is_icon) {
        buffer.set_text(&spec.text, &default_attrs, Shaping::Advanced, None);
        return;
    }

    buffer.set_rich_text(
        spans.into_iter().map(|(range, is_icon)| {
            let attrs = if is_icon {
                Attrs::new().family(font_family(FontFace::Icons))
            } else {
                Attrs::new()
                    .family(font_family(spec.face))
                    .weight(face_weight(spec.face))
            };
            (&spec.text[range], attrs)
        }),
        &default_attrs,
        Shaping::Advanced,
        None,
    );
}

/// Weight for a sans face variant; non-sans faces stay at normal weight.
fn face_weight(face: FontFace) -> Weight {
    match face {
        FontFace::SansMedium => Weight::MEDIUM,
        FontFace::SansBold => Weight::SEMIBOLD,
        _ => Weight::NORMAL,
    }
}

fn font_family(face: FontFace) -> Family<'static> {
    match face {
        FontFace::Sans | FontFace::SansMedium | FontFace::SansBold => {
            Family::Name("Instrument Sans")
        }
        FontFace::Icons | FontFace::Monospace | FontFace::Terminal => {
            Family::Name("JetBrainsMono Nerd Font Mono")
        }
    }
}

/// Builds a font system with the embedded application and mono faces registered.
///
/// `Instrument Sans` carries human text; `JetBrainsMono Nerd Font Mono` carries
/// icons, microlabels, code, and the terminal grid with Nerd Font glyph
/// coverage. System fonts remain as fallback.
fn brand_font_system() -> FontSystem {
    let mut font_system = FontSystem::new();
    let db = font_system.db_mut();
    db.load_font_data(include_bytes!("../../assets/fonts/InstrumentSans.ttf").to_vec());
    db.load_font_data(
        include_bytes!("../../assets/fonts/JetBrainsMonoNerdFontMono-Regular.ttf").to_vec(),
    );
    font_system
}

fn frame_pass<'a>(
    encoder: &'a mut wgpu::CommandEncoder,
    target: &'a wgpu::TextureView,
    label: &'static str,
    load: wgpu::LoadOp<wgpu::Color>,
) -> wgpu::RenderPass<'a> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load,
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    })
}

fn collect_rectangles(
    scene: &Scene,
) -> (
    Vec<RectInstance>,
    [Range<u32>; LAYER_COUNT],
    Vec<RectInstance>,
    [Range<u32>; LAYER_COUNT],
) {
    let total = scene
        .layers
        .iter()
        .map(|layer| layer.rectangles.len())
        .sum();
    let mut instances = Vec::with_capacity(total);
    let mut frost_instances = Vec::new();
    let mut ranges = std::array::from_fn(|_| 0..0);
    let mut frost_ranges = std::array::from_fn(|_| 0..0);
    for (index, layer) in scene.layers.iter().enumerate() {
        let start = instances.len().to_u32().unwrap_or(u32::MAX);
        let frost_start = frost_instances.len().to_u32().unwrap_or(u32::MAX);
        for rectangle in &layer.rectangles {
            let instance = RectInstance {
                rect: [
                    rectangle.rect.x,
                    rectangle.rect.y,
                    rectangle.rect.width,
                    rectangle.rect.height,
                ],
                clip: [
                    rectangle.clip.x,
                    rectangle.clip.y,
                    rectangle.clip.right(),
                    rectangle.clip.bottom(),
                ],
                fill: linear_color(rectangle.fill),
                border: linear_color(rectangle.border),
                params: [
                    rectangle.radius,
                    rectangle.border_width,
                    0.0,
                    rectangle.softness,
                ],
            };
            if rectangle.frost {
                frost_instances.push(instance);
            } else {
                instances.push(instance);
            }
        }
        ranges[index] = start..instances.len().to_u32().unwrap_or(u32::MAX);
        frost_ranges[index] = frost_start..frost_instances.len().to_u32().unwrap_or(u32::MAX);
    }
    (instances, ranges, frost_instances, frost_ranges)
}

fn collect_mesh(scene: &Scene) -> (Vec<GpuMeshVertex>, [Range<u32>; LAYER_COUNT]) {
    let total = scene.layers.iter().map(|layer| layer.mesh.len()).sum();
    let mut vertices = Vec::with_capacity(total);
    let mut ranges = std::array::from_fn(|_| 0..0);
    for (index, layer) in scene.layers.iter().enumerate() {
        let start = vertices.len().to_u32().unwrap_or(u32::MAX);
        vertices.extend(layer.mesh.iter().map(|vertex| GpuMeshVertex {
            position: vertex.position,
            color: linear_color(vertex.color),
            clip: [
                vertex.clip.x,
                vertex.clip.y,
                vertex.clip.right(),
                vertex.clip.bottom(),
            ],
        }));
        ranges[index] = start..vertices.len().to_u32().unwrap_or(u32::MAX);
    }
    (vertices, ranges)
}

fn color_target(format: wgpu::TextureFormat) -> wgpu::ColorTargetState {
    wgpu::ColorTargetState {
        format,
        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
        write_mask: wgpu::ColorWrites::ALL,
    }
}

fn linear_color(color: Color) -> [f32; 4] {
    [
        srgb_to_linear(color.0[0]),
        srgb_to_linear(color.0[1]),
        srgb_to_linear(color.0[2]),
        color.0[3],
    ]
}

fn srgb_to_linear(component: f32) -> f32 {
    if component <= 0.040_45 {
        component / 12.92
    } else {
        ((component + 0.055) / 1.055).powf(2.4)
    }
}

fn to_wgpu_color(color: Color) -> wgpu::Color {
    let value = linear_color(color);
    wgpu::Color {
        r: f64::from(value[0]),
        g: f64::from(value[1]),
        b: f64::from(value[2]),
        a: f64::from(value[3]),
    }
}

fn to_glyph_color(color: Color) -> GlyphColor {
    let channel = |value: f32| {
        (value.clamp(0.0, 1.0) * 255.0)
            .round()
            .to_u8()
            .unwrap_or(255)
    };
    GlyphColor::rgba(
        channel(color.0[0]),
        channel(color.0[1]),
        channel(color.0[2]),
        channel(color.0[3]),
    )
}
