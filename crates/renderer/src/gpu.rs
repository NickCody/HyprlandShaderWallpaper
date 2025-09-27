use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use bytemuck::{Pod, Zeroable};
use chrono::{Datelike, Local, Timelike};
use image::imageops::flip_vertical_in_place;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use wgpu::util::{DeviceExt, TextureDataOrder};
use wgpu::TextureFormatFeatureFlags;
use winit::dpi::PhysicalSize;

use crate::compile::{compile_fragment_shader, compile_vertex_shader};
use crate::types::{Antialiasing, ChannelBindings, ChannelSource, CHANNEL_COUNT};

pub(crate) struct GpuState {
    _instance: wgpu::Instance,
    limits: wgpu::Limits,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    size: PhysicalSize<u32>,
    sample_count: u32,
    multisample_target: Option<MultisampleTarget>,
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    #[allow(dead_code)]
    uniform_layout: wgpu::BindGroupLayout,
    #[allow(dead_code)]
    channel_layout: wgpu::BindGroupLayout,
    #[allow(dead_code)]
    pipeline_layout: wgpu::PipelineLayout,
    #[allow(dead_code)]
    vertex_module: wgpu::ShaderModule,
    uniforms: ShadertoyUniforms,
    current: ShaderPipeline,
    previous: Option<ShaderPipeline>,
    crossfade: Option<CrossfadeState>,
    start_time: Instant,
    last_frame_time: Instant,
    frame_count: u32,
    last_log_time: Instant,
}

impl GpuState {
    pub(crate) fn new<T>(
        target: &T,
        initial_size: PhysicalSize<u32>,
        shader_source: &Path,
        channel_bindings: &ChannelBindings,
        antialiasing: Antialiasing,
    ) -> Result<Self>
    where
        T: HasDisplayHandle + HasWindowHandle,
    {
        let instance = wgpu::Instance::default();
        let window_handle = target
            .window_handle()
            .map_err(|err| anyhow!("failed to acquire window handle: {err}"))?;
        let display_handle = target
            .display_handle()
            .map_err(|err| anyhow!("failed to acquire display handle: {err}"))?;
        let raw_window_handle = window_handle.as_raw();
        let raw_display_handle = display_handle.as_raw();
        let surface = unsafe {
            instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle,
                raw_window_handle,
            })
        }
        .context("failed to create rendering surface")?;

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .context("failed to find a suitable GPU adapter")?;

        let adapter_features = adapter.features();
        let limits = adapter.limits();
        let max_dimension = limits.max_texture_dimension_2d;
        let requested_width = initial_size.width.max(1);
        let requested_height = initial_size.height.max(1);
        if requested_width > max_dimension || requested_height > max_dimension {
            anyhow::bail!(
                "GPU max texture dimension is {max_dimension}, requested surface is {width}x{height}",
                max_dimension = max_dimension,
                width = requested_width,
                height = requested_height
            );
        }

        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|format| format.is_srgb())
            .unwrap_or(surface_caps.formats[0]);

        let format_features = adapter.get_texture_format_features(surface_format);
        let mut supported_samples = format_features.flags.supported_sample_counts();
        if !supported_samples.contains(&1) {
            supported_samples.push(1);
        }
        supported_samples.sort_unstable();
        supported_samples.dedup();

        let mut sample_count = match antialiasing {
            Antialiasing::Auto => *supported_samples.last().unwrap_or(&1),
            Antialiasing::Off => 1,
            Antialiasing::Samples(requested) => {
                if supported_samples.contains(&requested) {
                    requested
                } else {
                    let fallback = supported_samples
                        .iter()
                        .copied()
                        .filter(|&count| count <= requested)
                        .max()
                        .unwrap_or(*supported_samples.first().unwrap_or(&1));
                    tracing::warn!(
                        requested,
                        fallback,
                        ?supported_samples,
                        "requested MSAA sample count not supported; falling back"
                    );
                    fallback
                }
            }
        };

        if sample_count > 1
            && !format_features
                .flags
                .contains(TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE)
        {
            tracing::warn!(
                ?surface_format,
                "surface format does not support MSAA resolve; disabling MSAA"
            );
            sample_count = 1;
        }

        if sample_count > 4
            && !adapter_features.contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES)
        {
            let fallback = supported_samples
                .iter()
                .copied()
                .filter(|&count| count <= 4)
                .max()
                .unwrap_or(1);
            tracing::warn!(
                sample_count,
                fallback,
                "adapter lacks TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES; clamping MSAA"
            );
            sample_count = fallback;
        }

        tracing::info!(
            ?antialiasing,
            sample_count,
            supported_samples = ?supported_samples,
            "resolved MSAA configuration"
        );

        let mut required_features = wgpu::Features::empty();
        if sample_count > 4 {
            required_features |= wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES;
        }

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("shaderpaper device"),
            required_features,
            required_limits: limits.clone(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::default(),
        }))
        .context("failed to create GPU device")?;

        let size = PhysicalSize::new(requested_width, requested_height);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width,
            height: size.height,
            present_mode: surface_caps.present_modes[0],
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 1,
        };
        surface.configure(&device, &config);

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("uniform buffer"),
            size: std::mem::size_of::<ShadertoyUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

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

        let channel_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("channel layout"),
            entries: &build_channel_layout_entries(),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shader pipeline layout"),
            bind_group_layouts: &[&uniform_layout, &channel_layout],
            push_constant_ranges: &[],
        });

        let vertex_module = compile_vertex_shader(&device)?;

        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("uniform bind group"),
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let current = ShaderPipeline::new(
            &device,
            &queue,
            &pipeline_layout,
            &channel_layout,
            &vertex_module,
            surface_format,
            sample_count,
            shader_source,
            channel_bindings,
        )?;

        let uniforms = ShadertoyUniforms::new(size.width, size.height);
        queue.write_buffer(&uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        let multisample_target = if sample_count > 1 {
            Some(MultisampleTarget::new(
                &device,
                surface_format,
                size,
                sample_count,
            ))
        } else {
            None
        };

        Ok(Self {
            _instance: instance,
            limits,
            surface,
            device,
            queue,
            config,
            size,
            sample_count,
            multisample_target,
            uniform_buffer,
            uniform_bind_group,
            uniform_layout,
            channel_layout,
            pipeline_layout,
            vertex_module,
            uniforms,
            current,
            previous: None,
            crossfade: None,
            start_time: Instant::now(),
            last_frame_time: Instant::now(),
            frame_count: 0,
            last_log_time: Instant::now(),
        })
    }

    pub(crate) fn size(&self) -> PhysicalSize<u32> {
        self.size
    }

    pub(crate) fn resize(&mut self, new_size: PhysicalSize<u32>) {
        if new_size.width == 0 || new_size.height == 0 {
            return;
        }

        let max_dimension = self.limits.max_texture_dimension_2d;
        if new_size.width > max_dimension || new_size.height > max_dimension {
            tracing::warn!(
                new_width = new_size.width,
                new_height = new_size.height,
                max_dimension,
                old_width = self.size.width,
                old_height = self.size.height,
                "requested resize exceeds GPU limits; keeping previous size"
            );
            return;
        }

        self.size = new_size;
        self.config.width = new_size.width;
        self.config.height = new_size.height;
        self.surface.configure(&self.device, &self.config);
        self.multisample_target = if self.sample_count > 1 {
            Some(MultisampleTarget::new(
                &self.device,
                self.config.format,
                new_size,
                self.sample_count,
            ))
        } else {
            None
        };
        self.uniforms
            .set_resolution(new_size.width as f32, new_size.height as f32);
    }

    #[allow(dead_code)]
    pub(crate) fn set_shader(
        &mut self,
        shader_source: &Path,
        channel_bindings: &ChannelBindings,
        crossfade: Duration,
        now: Instant,
    ) -> Result<()> {
        let new_pipeline = ShaderPipeline::new(
            &self.device,
            &self.queue,
            &self.pipeline_layout,
            &self.channel_layout,
            &self.vertex_module,
            self.config.format,
            self.sample_count,
            shader_source,
            channel_bindings,
        )?;

        let crossfade = if crossfade < Duration::from_millis(16) {
            Duration::ZERO
        } else {
            crossfade
        };

        tracing::info!(
            shader = %shader_source.display(),
            crossfade_ms = crossfade.as_millis(),
            "swapping shader pipeline"
        );

        if crossfade.is_zero() {
            self.current = new_pipeline;
            self.previous = None;
            self.crossfade = None;
        } else {
            let previous = std::mem::replace(&mut self.current, new_pipeline);
            self.previous = Some(previous);
            self.crossfade = Some(CrossfadeState::new(now, crossfade));
        }

        Ok(())
    }

    pub(crate) fn render(&mut self, mouse: [f32; 4]) -> Result<(), wgpu::SurfaceError> {
        let now = Instant::now();
        self.update_time(mouse, now);

        let mut mix_prev = None;
        if let Some(fade) = self.crossfade.as_mut() {
            if fade.is_finished(now) || self.previous.is_none() {
                self.previous = None;
                self.crossfade = None;
            } else {
                mix_prev = Some((fade.previous_mix(now), fade.current_mix(now)));
            }
        }

        let frame = self.surface.get_current_texture()?;
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("render encoder"),
            });

        let mut load = wgpu::LoadOp::Clear(wgpu::Color::BLACK);

        let mut previous_pipeline = self.previous.take();
        let current_pipeline_ptr = &self.current as *const ShaderPipeline;

        if let Some((prev_mix, curr_mix)) = mix_prev {
            if prev_mix > 0.0 {
                if let Some(prev) = previous_pipeline.as_ref() {
                    self.render_with_pipeline(&mut encoder, &view, prev, prev_mix, load);
                    load = wgpu::LoadOp::Load;
                }
            } else {
                previous_pipeline = None;
                self.crossfade = None;
            }

            if curr_mix > 0.0 {
                unsafe {
                    self.render_with_pipeline(
                        &mut encoder,
                        &view,
                        &*current_pipeline_ptr,
                        curr_mix,
                        load,
                    );
                }
            }

            if self
                .crossfade
                .as_ref()
                .map(|fade| fade.is_finished(now))
                .unwrap_or(false)
                || curr_mix >= 1.0
            {
                previous_pipeline = None;
                self.crossfade = None;
            }
        } else {
            unsafe {
                self.render_with_pipeline(&mut encoder, &view, &*current_pipeline_ptr, 1.0, load);
            }
        }

        self.previous = previous_pipeline;

        self.queue.submit(std::iter::once(encoder.finish()));
        frame.present();
        tracing::trace!(
            "presented frame size={}x{}",
            self.size.width,
            self.size.height
        );
        Ok(())
    }

    fn render_with_pipeline(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        pipeline: &ShaderPipeline,
        mix: f32,
        load: wgpu::LoadOp<wgpu::Color>,
    ) {
        if mix <= 0.0 {
            return;
        }

        for (index, resource) in pipeline.channel_resources.iter().enumerate() {
            self.uniforms.i_channel_resolution[index] = resource.resolution;
        }
        self.uniforms.i_fade = mix;
        self.queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&self.uniforms));

        let (attachment_view, resolve_target) = if self.sample_count > 1 {
            let msaa = self
                .multisample_target
                .as_ref()
                .expect("MSAA target missing despite sample_count > 1");
            (&msaa.view, Some(view))
        } else {
            (view, None)
        };

        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("render pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: attachment_view,
                depth_slice: None,
                resolve_target,
                ops: wgpu::Operations {
                    load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
        });
        render_pass.set_pipeline(&pipeline.pipeline);
        render_pass.set_bind_group(0, &self.uniform_bind_group, &[]);
        render_pass.set_bind_group(1, &pipeline.channel_bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }

    fn update_time(&mut self, mouse: [f32; 4], now: Instant) {
        if self.frame_count == 0 {
            self.start_time = now;
            self.last_frame_time = now;
        }
        let elapsed = now.duration_since(self.start_time);
        let delta = now.duration_since(self.last_frame_time);
        self.uniforms.i_time = elapsed.as_secs_f32();
        self.uniforms.i_time_delta = delta.as_secs_f32();
        self.uniforms.i_frame = self.frame_count as i32;
        for channel in &mut self.uniforms.i_channel_time {
            channel[0] = self.uniforms.i_time;
        }
        self.uniforms.i_resolution[3] = self.uniforms.i_time;
        self.uniforms.i_mouse = mouse;

        let local_now = Local::now();
        let seconds_since_midnight = local_now.num_seconds_from_midnight() as f32
            + local_now.nanosecond() as f32 / 1_000_000_000.0;
        self.uniforms.i_date = [
            local_now.year() as f32,
            local_now.month() as f32,
            local_now.day() as f32,
            seconds_since_midnight,
        ];
        self.last_frame_time = now;
        self.frame_count = self.frame_count.saturating_add(1);

        if now.duration_since(self.last_log_time) >= Duration::from_secs(1) {
            eprintln!(
                "[shaderpaper] iTime={:.3}s, iFrame={}, iMouse=({}, {}, {}, {}), res=({}, {})",
                self.uniforms.i_time,
                self.uniforms.i_frame,
                self.uniforms.i_mouse[0],
                self.uniforms.i_mouse[1],
                self.uniforms.i_mouse[2],
                self.uniforms.i_mouse[3],
                self.size.width,
                self.size.height
            );
            self.last_log_time = now;
        }
    }
}

struct ShaderPipeline {
    #[allow(dead_code)]
    shader_path: PathBuf,
    pipeline: wgpu::RenderPipeline,
    channel_bind_group: wgpu::BindGroup,
    channel_resources: Vec<ChannelResources>,
}

impl ShaderPipeline {
    fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipeline_layout: &wgpu::PipelineLayout,
        channel_layout: &wgpu::BindGroupLayout,
        vertex_module: &wgpu::ShaderModule,
        surface_format: wgpu::TextureFormat,
        sample_count: u32,
        shader_path: &Path,
        channel_bindings: &ChannelBindings,
    ) -> Result<Self> {
        let shader_code = std::fs::read_to_string(shader_path)
            .with_context(|| format!("failed to read shader at {}", shader_path.display()))?;
        let fragment_module =
            compile_fragment_shader(device, &shader_code).context("failed to compile shader")?;

        let channel_resources = create_channel_resources(device, queue, channel_bindings.slots())?;
        let mut channel_entries = Vec::with_capacity(CHANNEL_COUNT * 2);
        for (index, resource) in channel_resources.iter().enumerate() {
            channel_entries.push(wgpu::BindGroupEntry {
                binding: (index as u32) * 2,
                resource: wgpu::BindingResource::TextureView(&resource.view),
            });
            channel_entries.push(wgpu::BindGroupEntry {
                binding: (index as u32) * 2 + 1,
                resource: wgpu::BindingResource::Sampler(&resource.sampler),
            });
        }

        let channel_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("channel bind group"),
            layout: channel_layout,
            entries: &channel_entries,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shader pipeline"),
            layout: Some(pipeline_layout),
            vertex: wgpu::VertexState {
                module: vertex_module,
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
                ..Default::default()
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

        Ok(Self {
            shader_path: shader_path.to_path_buf(),
            pipeline,
            channel_bind_group,
            channel_resources,
        })
    }
}

struct CrossfadeState {
    start: Instant,
    duration: Duration,
}

impl CrossfadeState {
    #[allow(dead_code)]
    fn new(start: Instant, duration: Duration) -> Self {
        Self { start, duration }
    }

    fn progress(&self, now: Instant) -> f32 {
        if self.duration.is_zero() {
            1.0
        } else {
            ((now.saturating_duration_since(self.start).as_secs_f32())
                / self.duration.as_secs_f32())
            .clamp(0.0, 1.0)
        }
    }

    fn previous_mix(&self, now: Instant) -> f32 {
        1.0 - self.progress(now)
    }

    fn current_mix(&self, now: Instant) -> f32 {
        self.progress(now)
    }

    fn is_finished(&self, now: Instant) -> bool {
        self.progress(now) >= 1.0
    }
}

struct MultisampleTarget {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}

impl MultisampleTarget {
    fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        size: PhysicalSize<u32>,
        sample_count: u32,
    ) -> Self {
        let extent = wgpu::Extent3d {
            width: size.width.max(1),
            height: size.height.max(1),
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("msaa color buffer"),
            size: extent,
            mip_level_count: 1,
            sample_count,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            _texture: texture,
            view,
        }
    }
}

#[repr(C, align(8))]
#[derive(Clone, Copy)]
struct Std140Vec2 {
    value: [f32; 2],
}

unsafe impl Zeroable for Std140Vec2 {}
unsafe impl Pod for Std140Vec2 {}

#[repr(C, align(16))]
#[derive(Clone, Copy)]
struct ShadertoyUniforms {
    i_resolution: [f32; 4],
    i_time: f32,
    i_time_delta: f32,
    i_frame: i32,
    _padding0: f32,
    i_mouse: [f32; 4],
    i_date: [f32; 4],
    i_sample_rate: f32,
    i_fade: f32,
    _padding1: Std140Vec2,
    i_channel_time: [[f32; 4]; CHANNEL_COUNT],
    i_channel_resolution: [[f32; 4]; CHANNEL_COUNT],
}

unsafe impl Zeroable for ShadertoyUniforms {}
unsafe impl Pod for ShadertoyUniforms {}

impl ShadertoyUniforms {
    fn new(width: u32, height: u32) -> Self {
        let mut uniforms = Self {
            i_resolution: [width as f32, height as f32, 0.0, 0.0],
            i_time: 0.0,
            i_time_delta: 0.0,
            i_frame: 0,
            _padding0: 0.0,
            i_mouse: [0.0; 4],
            i_date: [0.0; 4],
            i_sample_rate: 44100.0,
            i_fade: 1.0,
            _padding1: Std140Vec2 { value: [0.0; 2] },
            i_channel_time: [[0.0; 4]; CHANNEL_COUNT],
            i_channel_resolution: [[0.0; 4]; CHANNEL_COUNT],
        };

        uniforms.set_resolution(width as f32, height as f32);
        uniforms
    }

    fn set_resolution(&mut self, width: f32, height: f32) {
        self.i_resolution[0] = width;
        self.i_resolution[1] = height;
    }
}

fn build_channel_layout_entries() -> Vec<wgpu::BindGroupLayoutEntry> {
    let mut entries = Vec::with_capacity(CHANNEL_COUNT * 2);
    for index in 0..CHANNEL_COUNT {
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: (index as u32) * 2,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
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

struct ChannelResources {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    sampler: wgpu::Sampler,
    resolution: [f32; 4],
}

fn create_channel_resources(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    bindings: &[Option<ChannelSource>; CHANNEL_COUNT],
) -> Result<Vec<ChannelResources>> {
    let mut resources = Vec::with_capacity(CHANNEL_COUNT);
    for (index, binding) in bindings.iter().enumerate() {
        let resource = match binding {
            Some(ChannelSource::Texture { path }) => {
                match load_texture_channel(device, queue, index, path) {
                    Ok(resource) => resource,
                    Err(err) => {
                        tracing::warn!(
                            channel = index,
                            path = %path.display(),
                            error = %err,
                            "failed to load texture channel; using placeholder"
                        );
                        create_placeholder_channel(device, queue, index as u32)?
                    }
                }
            }
            None => create_placeholder_channel(device, queue, index as u32)?,
        };
        resources.push(resource);
    }

    Ok(resources)
}

fn create_placeholder_channel(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    index: u32,
) -> Result<ChannelResources> {
    let data = [255u8, 255, 255, 255];
    let texture = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some(&format!("placeholder channel texture #{index}")),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        },
        TextureDataOrder::LayerMajor,
        &data,
    );

    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });

    Ok(ChannelResources {
        _texture: texture,
        view,
        sampler,
        resolution: [1.0, 1.0, 1.0, 0.0],
    })
}

fn load_texture_channel(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    index: usize,
    path: &Path,
) -> Result<ChannelResources> {
    let image = image::open(path).with_context(|| {
        format!(
            "failed to open texture for channel {} at {}",
            index,
            path.display()
        )
    })?;

    let mut rgba = image.to_rgba8();
    let width = rgba.width();
    let height = rgba.height();
    if width == 0 || height == 0 {
        anyhow::bail!(
            "texture at {} has zero extent ({}x{})",
            path.display(),
            width,
            height
        );
    }

    flip_vertical_in_place(&mut rgba);

    let texture = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some(&format!("channel texture #{index} ({})", path.display())),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        },
        TextureDataOrder::LayerMajor,
        rgba.as_raw(),
    );

    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });

    tracing::info!(
        channel = index,
        path = %path.display(),
        width,
        height,
        "loaded texture channel"
    );

    Ok(ChannelResources {
        _texture: texture,
        view,
        sampler,
        resolution: [width as f32, height as f32, 1.0, 0.0],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{align_of, size_of};
    use std::time::{Duration, Instant};

    #[test]
    fn shadertoy_uniforms_follow_std140_layout() {
        let uniforms = ShadertoyUniforms::new(1920, 1080);
        let base = &uniforms as *const _ as usize;

        assert_eq!(align_of::<ShadertoyUniforms>(), 16);
        assert_eq!(size_of::<ShadertoyUniforms>(), 208);
        assert_eq!((&uniforms.i_resolution as *const _ as usize) - base, 0);
        assert_eq!((&uniforms.i_time as *const _ as usize) - base, 16);
        assert_eq!((&uniforms.i_mouse as *const _ as usize) - base, 32);
        assert_eq!((&uniforms.i_date as *const _ as usize) - base, 48);
        assert_eq!((&uniforms.i_sample_rate as *const _ as usize) - base, 64);
        assert_eq!((&uniforms.i_fade as *const _ as usize) - base, 68);
        assert_eq!((&uniforms.i_channel_time as *const _ as usize) - base, 80);
        assert_eq!(
            (&uniforms.i_channel_resolution as *const _ as usize) - base,
            144
        );
    }

    #[test]
    fn crossfade_weights_sum_to_one() {
        let start = Instant::now();
        let fade = CrossfadeState::new(start, Duration::from_secs(2));
        let midpoint = start + Duration::from_secs(1);

        let prev = fade.previous_mix(midpoint);
        let curr = fade.current_mix(midpoint);

        assert!((prev + curr - 1.0).abs() < 1e-5);
        assert!((prev - 0.5).abs() < 1e-5);
        assert!((curr - 0.5).abs() < 1e-5);
    }

    #[test]
    fn zero_duration_crossfade_is_hard_cut() {
        let start = Instant::now();
        let fade = CrossfadeState::new(start, Duration::ZERO);
        let later = start + Duration::from_millis(5);

        assert!(fade.is_finished(start));
        assert!(fade.is_finished(later));
        assert_eq!(fade.previous_mix(later), 0.0);
        assert_eq!(fade.current_mix(later), 1.0);
    }
}
