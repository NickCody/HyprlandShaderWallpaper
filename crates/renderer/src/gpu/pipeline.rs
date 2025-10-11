use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::compile::{compile_fragment_shader, compile_vertex_shader};
use crate::types::{ChannelBindings, ChannelTextureKind, ShaderCompiler, CHANNEL_COUNT};

use super::channels::{self, ChannelResources};
use super::context::SurfaceColorSpace;

pub(crate) struct PipelineLayouts {
    pub uniform_layout: wgpu::BindGroupLayout,
    pub vertex_module: wgpu::ShaderModule,
}

impl PipelineLayouts {
    pub fn new(device: &wgpu::Device, shader_compiler: ShaderCompiler) -> Result<Self> {
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("uniform layout"),
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

        let vertex_module = compile_vertex_shader(device, shader_compiler)?;

        Ok(Self {
            uniform_layout,
            vertex_module,
        })
    }
}

impl Clone for PipelineLayouts {
    fn clone(&self) -> Self {
        Self {
            uniform_layout: self.uniform_layout.clone(),
            vertex_module: self.vertex_module.clone(),
        }
    }
}

pub(crate) struct ShaderPipeline {
    pub pipeline: wgpu::RenderPipeline,
    pub channel_bind_group: wgpu::BindGroup,
    pub channel_resources: Vec<ChannelResources>,
    pub _channel_layout: wgpu::BindGroupLayout,
    has_keyboard: bool,
    pub shader_source: PathBuf,
}

impl ShaderPipeline {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layouts: &PipelineLayouts,
        surface_format: wgpu::TextureFormat,
        sample_count: u32,
        shader_path: &Path,
        channel_bindings: &ChannelBindings,
        channel_kinds: &[ChannelTextureKind; CHANNEL_COUNT],
        color_space: SurfaceColorSpace,
        shader_compiler: ShaderCompiler,
    ) -> Result<Self> {
        let shader_code = std::fs::read_to_string(shader_path)
            .with_context(|| format!("failed to read shader at {}", shader_path.display()))?;
        let fragment_module = compile_fragment_shader(device, &shader_code, shader_compiler)
            .context("failed to compile shader")?;

        let channel_resources = channels::create_resources(
            device,
            queue,
            channel_bindings,
            channel_kinds,
            color_space,
        )?;
        let channel_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("channel layout"),
            entries: &build_channel_layout_entries(channel_kinds),
        });
        let channel_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("channel bind group"),
            layout: &channel_layout,
            entries: &build_channel_entries(&channel_resources),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shader pipeline layout"),
            bind_group_layouts: &[&layouts.uniform_layout, &channel_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shader pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &layouts.vertex_module,
                entry_point: Some("main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: sample_count,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            fragment: Some(wgpu::FragmentState {
                module: &fragment_module,
                entry_point: Some("main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            multiview: None,
            cache: None,
        });

        let has_keyboard = channel_resources
            .iter()
            .any(|resource| resource.is_keyboard());

        Ok(Self::from_parts(
            pipeline,
            channel_bind_group,
            channel_resources,
            channel_layout,
            has_keyboard,
            shader_path.to_path_buf(),
        ))
    }

    pub(crate) fn from_parts(
        pipeline: wgpu::RenderPipeline,
        channel_bind_group: wgpu::BindGroup,
        channel_resources: Vec<ChannelResources>,
        channel_layout: wgpu::BindGroupLayout,
        has_keyboard: bool,
        shader_source: PathBuf,
    ) -> Self {
        Self {
            pipeline,
            channel_bind_group,
            channel_resources,
            _channel_layout: channel_layout,
            has_keyboard,
            shader_source,
        }
    }

    pub fn has_keyboard_channel(&self) -> bool {
        self.has_keyboard
    }

    pub fn update_keyboard_channels(&self, queue: &wgpu::Queue, data: &[u8]) {
        if !self.has_keyboard_channel() {
            return;
        }
        for resource in &self.channel_resources {
            resource.update_keyboard(queue, data);
        }
    }
}

pub(crate) fn build_channel_entries(
    resources: &[ChannelResources],
) -> Vec<wgpu::BindGroupEntry<'_>> {
    let mut entries = Vec::with_capacity(resources.len() * 2);
    for (index, resource) in resources.iter().enumerate() {
        entries.push(wgpu::BindGroupEntry {
            binding: (index as u32) * 2,
            resource: wgpu::BindingResource::TextureView(&resource.view),
        });
        entries.push(wgpu::BindGroupEntry {
            binding: (index as u32) * 2 + 1,
            resource: wgpu::BindingResource::Sampler(&resource.sampler),
        });
    }
    entries
}

pub(crate) fn build_channel_layout_entries(
    kinds: &[ChannelTextureKind; CHANNEL_COUNT],
) -> Vec<wgpu::BindGroupLayoutEntry> {
    let mut entries = Vec::with_capacity(CHANNEL_COUNT * 2);
    for (index, kind) in kinds.iter().enumerate() {
        let dimension = match kind {
            ChannelTextureKind::Texture2d => wgpu::TextureViewDimension::D2,
            ChannelTextureKind::Cubemap => wgpu::TextureViewDimension::Cube,
        };
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: (index as u32) * 2,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: dimension,
                multisampled: false,
            },
            count: None,
        });
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: (index as u32) * 2 + 1,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        });
    }
    entries
}
