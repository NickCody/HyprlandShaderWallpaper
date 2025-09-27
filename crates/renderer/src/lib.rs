//! Renderer crate for ShaderPaper (Hyprland Shader Wallpaper).
//!
//! The module glues the Wayland preview window, `wgpu` rendering pipeline, and
//! ShaderToy-compatible shader wrapping together. The overall flow is:
//!
//! ```text
//!   CLI / hyshadew
//!          │ RendererConfig
//!          ▼
//!   Renderer::run ──▶ WindowState ──▶ winit event loop ──▶ render_frame()
//!          ▲                                      │
//!          │                                      └─▶ update_uniforms() ─▶ GPU UBO
//! ```
//!
//! `WindowState` owns all GPU resources (surface, device, pipeline, uniforms),
//! while `Renderer` is the thin entry point that chooses between wallpaper mode
//! (currently a stub) or the interactive preview window. The fragment shaders
//! downloaded from ShaderToy are wrapped at runtime so they can be compiled as
//! Vulkan GLSL and fed the expected uniforms and texture bindings.

use std::borrow::Cow;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use bytemuck::{Pod, Zeroable};
use chrono::{Datelike, Local, Timelike};
use image::imageops::flip_vertical_in_place;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use wgpu::naga::ShaderStage;
use wgpu::util::{DeviceExt, TextureDataOrder};
use wgpu::TextureFormatFeatureFlags;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, Event, MouseButton, WindowEvent};
use winit::event_loop::{ControlFlow, EventLoop};
use winit::window::{Window, WindowBuilder};

/// ShaderToy exposes four optional input channels (`iChannel0-3`).
const CHANNEL_COUNT: usize = 4;

/// Describes how a ShaderToy channel should be populated.
#[derive(Clone, Debug)]
pub enum ChannelSource {
    Texture { path: PathBuf },
}

/// Collection of ShaderToy channel bindings prepared for the renderer.
#[derive(Clone, Debug)]
pub struct ChannelBindings {
    sources: [Option<ChannelSource>; CHANNEL_COUNT],
}

impl ChannelBindings {
    /// Creates an empty bindings table with all channels unassigned.
    pub fn new() -> Self {
        Self::default()
    }

    /// Associates a texture path with the given channel.
    pub fn set_texture(&mut self, channel: usize, path: PathBuf) -> Result<()> {
        if channel >= CHANNEL_COUNT {
            anyhow::bail!(
                "channel {} exceeds supported ShaderToy channel count ({})",
                channel,
                CHANNEL_COUNT
            );
        }
        self.sources[channel] = Some(ChannelSource::Texture { path });
        Ok(())
    }

    /// Exposes the underlying channel slots for GPU resource creation.
    fn slots(&self) -> &[Option<ChannelSource>; CHANNEL_COUNT] {
        &self.sources
    }
}

impl Default for ChannelBindings {
    fn default() -> Self {
        Self {
            sources: std::array::from_fn(|_| None),
        }
    }
}

/// How the renderer should present frames.
///
/// * `Wallpaper` will eventually stream frames directly into a Wayland layer
///   or xdg-shell surface owned by the compositor. This path is not implemented
///   yet, so we politely warn the caller instead of silently failing.
/// * `Windowed` spins up an interactive preview window driven by `winit` so we
///   can debug shaders on a desktop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderMode {
    Wallpaper,
    Windowed,
}

/// Anti-aliasing policy for the render pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Antialiasing {
    /// Pick the highest sample count supported by the surface format.
    Auto,
    /// Disable MSAA and render directly into the swapchain.
    Off,
    /// Request a specific MSAA sample count (clamped to what the device supports).
    Samples(u32),
}

impl Default for Antialiasing {
    fn default() -> Self {
        Self::Auto
    }
}

/// Immutable configuration passed to the renderer at start-up.
///
/// `RendererConfig` mirrors CLI flags and tells the renderer which shader file
/// to compile, how large the target surface should be, and which presentation
/// mode to use.
#[derive(Clone)]
pub struct RendererConfig {
    /// Window or surface size in physical pixels.
    pub surface_size: (u32, u32),
    /// Path to the ShaderToy-style fragment shader that should be rendered.
    pub shader_source: PathBuf,
    /// Presentation mode (wallpaper vs interactive window).
    pub mode: RenderMode,
    /// Optional resolution explicitly requested by the caller.
    pub requested_size: Option<(u32, u32)>,
    /// Optional FPS cap for wallpaper mode; None = render every callback.
    pub target_fps: Option<f32>,
    /// Optional ShaderToy channel bindings (textures, cubemaps, etc.).
    pub channel_bindings: ChannelBindings,
    /// Anti-aliasing mode requested by the caller.
    pub antialiasing: Antialiasing,
}

impl Default for RendererConfig {
    /// Provides a 1080p windowed configuration with no shader selected.
    fn default() -> Self {
        Self {
            surface_size: (1920, 1080),
            shader_source: PathBuf::new(),
            mode: RenderMode::Wallpaper,
            requested_size: None,
            target_fps: None,
            channel_bindings: ChannelBindings::default(),
            antialiasing: Antialiasing::default(),
        }
    }
}

/// High-level entry point that owns the chosen configuration.
///
/// The heavy lifting lives inside [`WindowState`]; `Renderer` simply selects the
/// presentation path and forwards the request.
pub struct Renderer {
    config: RendererConfig,
}

impl Renderer {
    /// Builds a renderer for the supplied configuration.
    pub fn new(config: RendererConfig) -> Self {
        Self { config }
    }

    /// Launches the renderer in either wallpaper or windowed mode.
    ///
    /// Returns an error if the mode fails to initialize (for example when the
    /// system lacks a Wayland compositor). Wallpaper mode is currently
    /// unimplemented, so we emit a friendly message and return success instead
    /// of surprising the caller.
    pub fn run(&mut self) -> Result<()> {
        match self.config.mode {
            RenderMode::Wallpaper => self.run_wallpaper(),
            RenderMode::Windowed => self.run_window_preview(),
        }
    }

    /// Drives the Wayland wallpaper path, rendering into a background layer surface.
    fn run_wallpaper(&self) -> Result<()> {
        wallpaper::run(&self.config)
    }

    /// Opens the preview window and drives the `winit` event loop.
    ///
    /// A `WindowState` is created up-front and stored inside the event loop
    /// closure. `winit` delivers events one by one; we respond to them and draw
    /// another frame whenever a redraw is requested.
    fn run_window_preview(&self) -> Result<()> {
        let event_loop = EventLoop::new().context("failed to initialize event loop")?;
        let window_size = PhysicalSize::new(self.config.surface_size.0, self.config.surface_size.1);
        let window = WindowBuilder::new()
            .with_title("Hyshadew Preview")
            .with_inner_size(window_size)
            .build(&event_loop)
            .context("failed to create preview window")?;
        let window = Arc::new(window);

        let mut state = WindowState::new(window.clone(), &self.config)?;
        state.window().request_redraw();

        event_loop
            .run(move |event, elwt| {
                // Drive redraws via vblank by waiting between events.
                elwt.set_control_flow(ControlFlow::Wait);

                match event {
                    Event::WindowEvent { window_id, event } if window_id == state.window().id() => {
                        match event {
                            WindowEvent::CloseRequested | WindowEvent::Destroyed => {
                                elwt.exit();
                            }
                            WindowEvent::CursorMoved { position, .. } => {
                                state.mouse.handle_cursor_moved(position);
                            }
                            WindowEvent::MouseInput {
                                state: button_state,
                                button,
                                ..
                            } => {
                                if button == MouseButton::Left {
                                    state.mouse.handle_button(button_state);
                                }
                            }
                            WindowEvent::Resized(new_size) => {
                                state.resize(new_size);
                            }
                            WindowEvent::ScaleFactorChanged {
                                mut inner_size_writer,
                                ..
                            } => {
                                // Keep the current logical size when the scale factor changes.
                                let _ = inner_size_writer.request_inner_size(state.size());
                            }
                            WindowEvent::RedrawRequested => {
                                let render_result = state.render_frame();
                                match render_result {
                                    Ok(()) => {}
                                    Err(
                                        wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated,
                                    ) => {
                                        state.resize(state.size());
                                    }
                                    Err(wgpu::SurfaceError::OutOfMemory) => {
                                        eprintln!("surface out of memory; exiting");
                                        elwt.exit();
                                    }
                                    Err(wgpu::SurfaceError::Timeout) => {
                                        eprintln!("surface timeout; retrying next frame");
                                    }
                                    Err(other) => {
                                        eprintln!("surface error: {other:?}; retrying next frame");
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    Event::AboutToWait => {
                        // Schedule the next frame once winit is about to wait for events again.
                        state.window().request_redraw();
                    }
                    _ => {}
                }
            })
            .map_err(|err| anyhow!("event loop error: {err}"))
    }
}

/// Aggregates every GPU resource needed to present a frame.
///
/// The layout intentionally mirrors the lifetime relationship between objects:
///
/// ```text
///   Window ─┐
///           ├─▶ Surface ─▶ Device ─▶ Queue
///           │                 │
///           │                 ├─▶ RenderPipeline
///           │                 ├─▶ Buffers (uniforms)
///           │                 └─▶ Bind groups / textures
/// ```
///
/// `WindowState` is parameterised over the shader we compile at runtime and is
/// shared with the event loop so each frame can mutate the uniforms, draw, and
/// react to input.
struct WindowState {
    /// Shared handle to the Wayland window (`wgpu` requires it to create the surface).
    window: Arc<Window>,
    /// GPU resources backing the swapchain and shader pipeline.
    gpu: GpuState,
    /// Mouse tracking helper for `iMouse`.
    mouse: MouseState,
}

impl WindowState {
    /// Creates a fully initialised rendering state for the preview window.
    ///
    /// The method configures the swapchain, compiles the ShaderToy fragment
    /// shader, builds the render pipeline, and seeds the uniform buffer. The
    /// heavy lifting happens synchronously after `wgpu` gives us access to the
    /// adapter and device.
    fn new(window: Arc<Window>, config: &RendererConfig) -> Result<Self> {
        let size = window.inner_size();
        let gpu = GpuState::new(
            window.as_ref(),
            size,
            &config.shader_source,
            &config.channel_bindings,
            config.antialiasing,
        )?;

        Ok(Self {
            window,
            gpu,
            mouse: MouseState::default(),
        })
    }

    fn window(&self) -> &Window {
        self.window.as_ref()
    }

    /// Cached physical size of the swapchain surface.
    fn size(&self) -> PhysicalSize<u32> {
        self.gpu.size()
    }

    /// Reacts to platform resize events by updating the swapchain and uniforms.
    fn resize(&mut self, new_size: PhysicalSize<u32>) {
        self.gpu.resize(new_size);
    }

    /// Records and submits a frame to the GPU.
    fn render_frame(&mut self) -> Result<(), wgpu::SurfaceError> {
        let mouse_uniform = self.mouse.as_uniform(self.size().height.max(1) as f32);
        self.gpu.render_frame(mouse_uniform)
    }
}

/// Owns the GPU resources required to render the ShaderToy pipeline.
struct GpuState {
    /// `wgpu` instance that produced the surface; kept alive for the surface lifetime.
    _instance: wgpu::Instance,
    /// Limits advertised by the adapter; used to validate resize requests.
    limits: wgpu::Limits,
    /// Swapchain surface we render into each frame.
    surface: wgpu::Surface<'static>,
    /// Logical device used for resource creation.
    device: wgpu::Device,
    /// Submission queue accepting command buffers.
    queue: wgpu::Queue,
    /// Swapchain configuration (format, present mode, dimensions).
    config: wgpu::SurfaceConfiguration,
    /// Current swapchain size in physical pixels.
    size: PhysicalSize<u32>,
    /// MSAA sample count used by the render pipeline.
    sample_count: u32,
    /// Optional multisample color buffer when MSAA is enabled.
    multisample_target: Option<MultisampleTarget>,
    /// Full-screen pipeline driving the fragment shader.
    pipeline: wgpu::RenderPipeline,
    /// GPU buffer containing the ShaderToy uniform block.
    uniform_buffer: wgpu::Buffer,
    /// Bind group that exposes the uniform buffer to the shader.
    uniform_bind_group: wgpu::BindGroup,
    /// Bind group containing channel textures/samplers.
    channel_bind_group: wgpu::BindGroup,
    /// Owned textures/samplers so the bind group remains valid.
    _channel_resources: Vec<ChannelResources>,
    /// CPU copy of the uniform data mirrored into the buffer each frame.
    uniforms: ShadertoyUniforms,
    /// Instant captured when rendering begins.
    start_time: Instant,
    /// Timestamp of the previously presented frame.
    last_frame_time: Instant,
    /// Monotonic frame counter used for `iFrame`.
    frame_count: u32,
    /// Used to throttle debug logging.
    last_log_time: Instant,
}

impl GpuState {
    /// Creates a GPU pipeline targeting the supplied surface and size.
    fn new<T>(
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

        let device_descriptor = wgpu::DeviceDescriptor {
            label: Some("shaderpaper device"),
            required_features,
            required_limits: limits.clone(),
            memory_hints: wgpu::MemoryHints::default(),
            trace: wgpu::Trace::default(),
        };

        let (device, queue) = pollster::block_on(adapter.request_device(&device_descriptor))
            .context("failed to create GPU device")?;

        let size = PhysicalSize::new(requested_width, requested_height);
        tracing::info!(
            "initial surface size {}x{}, max_texture_dimension_2d={max_dimension}",
            requested_width,
            requested_height
        );

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

        let shader_code = fs::read_to_string(shader_source)
            .with_context(|| format!("failed to read shader at {}", shader_source.display()))?;

        let fragment_module = compile_fragment_shader(&device, &shader_code)
            .context("failed to compile shader fragment")?;
        let vertex_module = compile_vertex_shader(&device)?;

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

        let mut channel_layout_entries = Vec::with_capacity(CHANNEL_COUNT * 2);
        for index in 0..CHANNEL_COUNT {
            channel_layout_entries.push(wgpu::BindGroupLayoutEntry {
                binding: (index as u32) * 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            });
            channel_layout_entries.push(wgpu::BindGroupLayoutEntry {
                binding: (index as u32) * 2 + 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            });
        }

        let channel_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("channel layout"),
            entries: &channel_layout_entries,
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shader pipeline layout"),
            bind_group_layouts: &[&uniform_layout, &channel_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shader pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &vertex_module,
                entry_point: Some("main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: sample_count,
                ..wgpu::MultisampleState::default()
            },
            fragment: Some(wgpu::FragmentState {
                module: &fragment_module,
                entry_point: Some("main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            multiview: None,
            cache: None,
        });

        let mut uniforms = ShadertoyUniforms::new(size.width, size.height);

        let channel_resources =
            create_channel_resources(&device, &queue, channel_bindings.slots())?;
        let mut channel_entries = Vec::with_capacity(CHANNEL_COUNT * 2);
        for (index, resource) in channel_resources.iter().enumerate() {
            uniforms.i_channel_resolution[index] = resource.resolution;
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
            layout: &channel_layout,
            entries: &channel_entries,
        });

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

        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("uniform buffer"),
            contents: bytemuck::bytes_of(&uniforms),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("uniform bind group"),
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

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
            pipeline,
            uniform_buffer,
            uniform_bind_group,
            channel_bind_group,
            _channel_resources: channel_resources,
            uniforms,
            start_time: Instant::now(),
            last_frame_time: Instant::now(),
            frame_count: 0,
            last_log_time: Instant::now(),
        })
    }

    /// Returns the current surface size.
    fn size(&self) -> PhysicalSize<u32> {
        self.size
    }

    /// Reconfigures the swapchain to match the new size.
    fn resize(&mut self, new_size: PhysicalSize<u32>) {
        if new_size.width == 0 || new_size.height == 0 {
            return;
        }

        let max_dimension = self.limits.max_texture_dimension_2d;
        if new_size.width > max_dimension || new_size.height > max_dimension {
            eprintln!(
                "requested resize to {new_width}x{new_height} exceeds GPU max texture dimension {max_dimension}; keeping previous size {old_width}x{old_height}",
                new_width = new_size.width,
                new_height = new_size.height,
                max_dimension = max_dimension,
                old_width = self.size.width,
                old_height = self.size.height
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

    /// Advances the ShaderToy uniform block and uploads it to the GPU.
    fn update_uniforms(&mut self, mouse: [f32; 4]) {
        let now = Instant::now();
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
        // Mirror time into the spare resolution slot to paper over drivers that drop std140 padding.
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
        self.queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&self.uniforms));

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

    /// Records and submits a frame to the GPU.
    fn render_frame(&mut self, mouse: [f32; 4]) -> Result<(), wgpu::SurfaceError> {
        self.update_uniforms(mouse);

        let frame = self.surface.get_current_texture()?;
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("render encoder"),
            });

        let (attachment_view, resolve_target) = if self.sample_count > 1 {
            let msaa = self
                .multisample_target
                .as_ref()
                .expect("multisample target should exist when MSAA is enabled");
            (&msaa.view, Some(&view))
        } else {
            (&view, None)
        };

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("render pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: attachment_view,
                    depth_slice: None,
                    resolve_target,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });
            render_pass.set_pipeline(&self.pipeline);
            render_pass.set_bind_group(0, &self.uniform_bind_group, &[]);
            render_pass.set_bind_group(1, &self.channel_bind_group, &[]);
            render_pass.draw(0..3, 0..1);
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        frame.present();
        tracing::trace!(
            "presented frame size={}x{}",
            self.size.width,
            self.size.height
        );
        Ok(())
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
            label: Some("shaderpaper msaa color"),
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

/// Builds 1×1 placeholder textures for the ShaderToy channels.
///
/// ShaderToy shaders expect `iChannelResolution` to contain meaningful data
/// even when no external texture is plugged in. We upload a single opaque pixel
/// per channel and remember the resolution so the uniform block stays
/// consistent.
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

/// Placeholder textures/samplers for the four ShaderToy channels.
struct ChannelResources {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    sampler: wgpu::Sampler,
    resolution: [f32; 4],
}

/// Convenience wrapper that forces a vec3-sized slot to occupy a full 16 bytes.
#[repr(C, align(16))]
/// Convenience wrapper that forces a vec3-sized slot to occupy a full 16 bytes.
#[repr(C, align(16))]
#[derive(Clone, Copy)]
struct Std140Vec3 {
    value: [f32; 3],
    _pad: f32,
}

impl Std140Vec3 {
    /// A zero-initialised constant used to seed uniform padding.
    const ZERO: Self = Self {
        value: [0.0; 3],
        _pad: 0.0,
    };
}

unsafe impl Zeroable for Std140Vec3 {}
unsafe impl Pod for Std140Vec3 {}

/// CPU-side mirror of the ShaderToy uniform block.
///
/// The layout matches the GLSL prelude injected by [`wrap_shadertoy_fragment`]
/// and therefore must observe std140 alignment rules. The fourth component of
/// `i_resolution` doubles as spare storage for `iTime` so GLSL front-ends that
/// drop vec3 padding still see an animating value.
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
    _padding1: Std140Vec3,
    i_channel_time: [[f32; 4]; CHANNEL_COUNT],
    i_channel_resolution: [[f32; 4]; CHANNEL_COUNT],
}

unsafe impl Zeroable for ShadertoyUniforms {}
unsafe impl Pod for ShadertoyUniforms {}

impl ShadertoyUniforms {
    /// Prepares a uniform block sized to the current surface.
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
            _padding1: Std140Vec3::ZERO,
            i_channel_time: [[0.0; 4]; CHANNEL_COUNT],
            i_channel_resolution: [[0.0; 4]; CHANNEL_COUNT],
        };

        uniforms.set_resolution(width as f32, height as f32);
        uniforms
    }

    /// Writes the current surface dimensions into `iResolution`.
    fn set_resolution(&mut self, width: f32, height: f32) {
        self.i_resolution[0] = width;
        self.i_resolution[1] = height;
    }
}

#[derive(Default)]
/// Tracks cursor motion and drag state so shaders receive ShaderToy-compatible
/// `iMouse` values.
struct MouseState {
    position: Option<PhysicalPosition<f64>>,
    pressed_anchor: Option<PhysicalPosition<f64>>,
    is_pressed: bool,
}

impl MouseState {
    /// Records the latest cursor position.
    fn handle_cursor_moved(&mut self, position: PhysicalPosition<f64>) {
        self.position = Some(position);
        if self.is_pressed {
            self.pressed_anchor.get_or_insert(position);
        }
    }

    /// Notes when the primary button transitions between pressed/released.
    fn handle_button(&mut self, state: ElementState) {
        match state {
            ElementState::Pressed => {
                self.is_pressed = true;
                if let Some(pos) = self.position {
                    self.pressed_anchor = Some(pos);
                }
            }
            ElementState::Released => {
                self.is_pressed = false;
                self.pressed_anchor = None;
            }
        }
    }

    /// Produces the four floats expected by ShaderToy's `iMouse` uniform.
    fn as_uniform(&self, height: f32) -> [f32; 4] {
        let mut data = [0.0; 4];

        if let Some(pos) = self.position {
            data[0] = pos.x as f32;
            data[1] = height - pos.y as f32;
        }

        if let Some(anchor) = self.pressed_anchor {
            data[2] = anchor.x as f32;
            data[3] = height - anchor.y as f32;
        }

        data
    }
}

/// Compiles the static full-screen triangle vertex shader.
fn compile_vertex_shader(device: &wgpu::Device) -> Result<wgpu::ShaderModule> {
    Ok(device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("fullscreen triangle vertex"),
        source: wgpu::ShaderSource::Glsl {
            shader: Cow::Borrowed(VERTEX_SHADER_GLSL),
            stage: ShaderStage::Vertex,
            defines: &[],
        },
    }))
}

mod wallpaper {
    use super::{Antialiasing, ChannelBindings, GpuState, RendererConfig};
    use anyhow::{Context, Result};
    use raw_window_handle::{
        DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
        RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle, WindowHandle,
    };
    use smithay_client_toolkit::reexports::client::{
        globals::registry_queue_init,
        protocol::{wl_output, wl_surface},
        Connection, Proxy, QueueHandle,
    };
    use smithay_client_toolkit::{
        compositor::{CompositorHandler, CompositorState},
        delegate_compositor, delegate_layer, delegate_output, delegate_registry,
        output::{OutputHandler, OutputInfo, OutputState},
        registry::{ProvidesRegistryState, RegistryState},
        registry_handlers,
        shell::wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
        shell::WaylandSurface,
    };
    use std::ffi::c_void;
    use std::path::PathBuf;
    use std::ptr::NonNull;
    use std::result::Result as StdResult;
    use std::time::{Duration, Instant};
    use winit::dpi::PhysicalSize;

    pub(super) fn run(config: &RendererConfig) -> Result<()> {
        let conn =
            Connection::connect_to_env().context("failed to connect to Wayland compositor")?;
        let (globals, mut event_queue) =
            registry_queue_init(&conn).context("failed to initialize Wayland registry queue")?;
        let qh = event_queue.handle();

        let compositor =
            CompositorState::bind(&globals, &qh).context("wl_compositor is not available")?;
        let layer_shell =
            LayerShell::bind(&globals, &qh).context("layer shell protocol is not available")?;

        let registry_state = RegistryState::new(&globals);
        let output_state = OutputState::new(&globals, &qh);

        let surface = compositor.create_surface(&qh);
        let target_output = output_state.outputs().next();

        let initial_output_size = target_output
            .as_ref()
            .and_then(|output| output_state.info(output))
            .and_then(output_info_physical_size);

        let layer_surface = layer_shell.create_layer_surface(
            &qh,
            surface,
            Layer::Background,
            Some("shaderpaper".to_string()),
            target_output.as_ref(),
        );
        layer_surface.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer_surface.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer_surface.set_exclusive_zone(-1);
        if let Some((width, height)) = config.requested_size {
            layer_surface.set_size(width, height);
        }
        layer_surface.commit();

        let fallback_size = PhysicalSize::new(config.surface_size.0, config.surface_size.1);
        let mut state = WallpaperState::new(
            registry_state,
            output_state,
            layer_surface,
            config.shader_source.clone(),
            config.requested_size,
            fallback_size,
            config.channel_bindings.clone(),
            config.antialiasing,
            target_output,
            initial_output_size,
        );

        // Configure FPS cap if requested.
        if let Some(fps) = config.target_fps {
            if fps > 0.0 {
                state.fps_cap = Some(fps);
                state.target_interval = Some(Duration::from_secs_f32(1.0 / fps));
                tracing::info!("wallpaper fps cap set to {:.1} FPS", fps);
            }
        }

        loop {
            event_queue
                .blocking_dispatch(&mut state)
                .context("error while processing Wayland events")?;
            if state.should_exit() {
                break;
            }
        }

        Ok(())
    }

    struct WallpaperState {
        registry_state: RegistryState,
        output_state: OutputState,
        layer_surface: LayerSurface,
        shader_source: PathBuf,
        requested_size: Option<(u32, u32)>,
        fallback_size: PhysicalSize<u32>,
        channel_bindings: ChannelBindings,
        antialiasing: Antialiasing,
        gpu: Option<GpuState>,
        frame_scheduled: bool,
        should_exit: bool,
        target_output: Option<wl_output::WlOutput>,
        last_output_size: Option<PhysicalSize<u32>>,
        // FPS cap state (None = uncapped)
        fps_cap: Option<f32>,
        target_interval: Option<Duration>,
        accumulator: Duration,
        last_tick: Option<Instant>,
    }

    impl WallpaperState {
        fn new(
            registry_state: RegistryState,
            output_state: OutputState,
            layer_surface: LayerSurface,
            shader_source: PathBuf,
            requested_size: Option<(u32, u32)>,
            fallback_size: PhysicalSize<u32>,
            channel_bindings: ChannelBindings,
            antialiasing: Antialiasing,
            target_output: Option<wl_output::WlOutput>,
            last_output_size: Option<PhysicalSize<u32>>,
        ) -> Self {
            Self {
                registry_state,
                output_state,
                layer_surface,
                shader_source,
                requested_size,
                fallback_size,
                channel_bindings,
                antialiasing,
                gpu: None,
                frame_scheduled: false,
                should_exit: false,
                target_output,
                last_output_size,
                fps_cap: None,
                target_interval: None,
                accumulator: Duration::ZERO,
                last_tick: None,
            }
        }

        fn should_exit(&self) -> bool {
            self.should_exit
        }

        fn ensure_gpu(&mut self, conn: &Connection, size: PhysicalSize<u32>) -> Result<bool> {
            if let Some(gpu) = self.gpu.as_mut() {
                gpu.resize(size);
                tracing::debug!("resized GPU surface to {}x{}", size.width, size.height);
                return Ok(false);
            }

            let handle = WaylandSurfaceHandle::new(conn, &self.layer_surface);
            let gpu = GpuState::new(
                &handle,
                size,
                &self.shader_source,
                &self.channel_bindings,
                self.antialiasing,
            )?;
            tracing::info!("initialised GPU surface {}x{}", size.width, size.height);
            self.gpu = Some(gpu);
            // Reset FPS pacing on (re)create
            self.accumulator = Duration::ZERO;
            self.last_tick = Some(Instant::now());
            Ok(true)
        }

        fn schedule_frame(&mut self, qh: &QueueHandle<Self>) {
            if self.frame_scheduled || self.gpu.is_none() {
                return;
            }
            let surface = self.layer_surface.wl_surface();
            surface.frame(qh, surface.clone());
            self.frame_scheduled = true;
            self.layer_surface.commit();
            tracing::trace!("requested frame callback and committed surface");
        }

        fn infer_output_size(&self) -> Option<PhysicalSize<u32>> {
            if let Some(output) = self.target_output.as_ref() {
                if let Some(info) = self.output_state.info(output) {
                    return output_info_physical_size(info);
                }
            }
            self.last_output_size
        }

        fn resolve_configure_size(&self, new_size: (u32, u32)) -> PhysicalSize<u32> {
            let mut size = if new_size.0 == 0 || new_size.1 == 0 {
                self.infer_output_size().unwrap_or(self.fallback_size)
            } else {
                PhysicalSize::new(new_size.0.max(1), new_size.1.max(1))
            };

            if let Some((req_w, req_h)) = self.requested_size {
                let req_w = req_w.max(1);
                let req_h = req_h.max(1);
                if req_w < size.width {
                    size.width = req_w;
                }
                if req_h < size.height {
                    size.height = req_h;
                }
            }

            if size.width == 0 || size.height == 0 {
                self.fallback_size
            } else {
                size
            }
        }

        fn refresh_output_size(&mut self, output: &wl_output::WlOutput) {
            if let Some(info) = self.output_state.info(output) {
                let physical = output_info_physical_size(info);
                if self
                    .target_output
                    .as_ref()
                    .map(|current| current == output)
                    .unwrap_or(false)
                {
                    self.last_output_size = physical;
                } else if self.target_output.is_none() {
                    self.target_output = Some(output.clone());
                    self.last_output_size = physical;
                }
            }
        }

        fn handle_surface_error(&mut self, error: wgpu::SurfaceError) {
            match error {
                wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated => {
                    if let Some(gpu) = self.gpu.as_mut() {
                        gpu.resize(gpu.size());
                    }
                }
                wgpu::SurfaceError::OutOfMemory => {
                    eprintln!("surface out of memory; exiting wallpaper loop");
                    self.should_exit = true;
                }
                wgpu::SurfaceError::Timeout => {
                    eprintln!("surface timeout; retrying next frame");
                }
                wgpu::SurfaceError::Other => {
                    eprintln!("surface reported an unknown error; retrying next frame");
                }
            }
        }
    }

    impl CompositorHandler for WallpaperState {
        fn scale_factor_changed(
            &mut self,
            _conn: &Connection,
            _qh: &QueueHandle<Self>,
            _surface: &wl_surface::WlSurface,
            _new_factor: i32,
        ) {
        }

        fn transform_changed(
            &mut self,
            _conn: &Connection,
            _qh: &QueueHandle<Self>,
            _surface: &wl_surface::WlSurface,
            _new_transform: wl_output::Transform,
        ) {
        }

        fn frame(
            &mut self,
            conn: &Connection,
            qh: &QueueHandle<Self>,
            surface: &wl_surface::WlSurface,
            _time: u32,
        ) {
            if surface != self.layer_surface.wl_surface() {
                return;
            }

            self.frame_scheduled = false;

            if let Some(gpu) = self.gpu.as_mut() {
                // FPS pacing
                let mut should_render = true;
                if let (Some(interval), Some(last)) = (self.target_interval, self.last_tick) {
                    let now = Instant::now();
                    let delta = now.saturating_duration_since(last);
                    self.last_tick = Some(now);
                    self.accumulator = self.accumulator.saturating_add(delta);
                    if self.accumulator + Duration::from_micros(250) < interval {
                        should_render = false;
                    } else {
                        // subtract only one interval to avoid burst on long gaps
                        self.accumulator = self.accumulator.saturating_sub(interval);
                    }
                }

                if should_render {
                    tracing::trace!("frame callback - rendering");
                    match gpu.render_frame([0.0; 4]) {
                        Ok(()) => {}
                        Err(err) => self.handle_surface_error(err),
                    }
                } else {
                    tracing::trace!("frame callback - skipped render due to fps cap");
                }
                // Always commit to keep callbacks flowing
                self.layer_surface.commit();
            } else if let Some(size) = self.infer_output_size() {
                let created = match self.ensure_gpu(conn, size) {
                    Ok(created) => created,
                    Err(err) => {
                        eprintln!("failed to initialize GPU for wallpaper: {err:?}");
                        self.should_exit = true;
                        return;
                    }
                };

                if created {
                    if let Some(gpu) = self.gpu.as_mut() {
                        tracing::trace!("rendering bootstrap frame after missing gpu");
                        if let Err(err) = gpu.render_frame([0.0; 4]) {
                            self.handle_surface_error(err);
                            return;
                        }
                        self.layer_surface.commit();
                        tracing::trace!("committed bootstrap frame");
                    }
                }
            }

            self.schedule_frame(qh);
        }
    }

    impl LayerShellHandler for WallpaperState {
        fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _layer: &LayerSurface) {
            self.should_exit = true;
        }

        fn configure(
            &mut self,
            conn: &Connection,
            qh: &QueueHandle<Self>,
            _layer: &LayerSurface,
            configure: LayerSurfaceConfigure,
            _serial: u32,
        ) {
            let size = self.resolve_configure_size(configure.new_size);
            self.layer_surface.set_size(size.width, size.height);
            self.last_output_size = Some(size);
            tracing::info!(
                "layer configure new_size={}x{} -> using {}x{}",
                configure.new_size.0,
                configure.new_size.1,
                size.width,
                size.height
            );

            let created = match self.ensure_gpu(conn, size) {
                Ok(created) => created,
                Err(err) => {
                    eprintln!("failed to prepare GPU for wallpaper: {err:?}");
                    self.should_exit = true;
                    return;
                }
            };

            if created {
                if let Some(gpu) = self.gpu.as_mut() {
                    tracing::trace!("rendering bootstrap frame");
                    if let Err(err) = gpu.render_frame([0.0; 4]) {
                        self.handle_surface_error(err);
                        return;
                    }
                    self.layer_surface.commit();
                    tracing::trace!("committed bootstrap frame");
                }
            }

            self.schedule_frame(qh);
        }
    }

    impl OutputHandler for WallpaperState {
        fn output_state(&mut self) -> &mut OutputState {
            &mut self.output_state
        }

        fn new_output(
            &mut self,
            _conn: &Connection,
            _qh: &QueueHandle<Self>,
            output: wl_output::WlOutput,
        ) {
            self.refresh_output_size(&output);
        }

        fn update_output(
            &mut self,
            _conn: &Connection,
            _qh: &QueueHandle<Self>,
            output: wl_output::WlOutput,
        ) {
            self.refresh_output_size(&output);
        }

        fn output_destroyed(
            &mut self,
            _conn: &Connection,
            _qh: &QueueHandle<Self>,
            output: wl_output::WlOutput,
        ) {
            if self
                .target_output
                .as_ref()
                .map(|current| current == &output)
                .unwrap_or(false)
            {
                self.target_output = None;
                self.last_output_size = None;
            }
        }
    }

    impl ProvidesRegistryState for WallpaperState {
        fn registry(&mut self) -> &mut RegistryState {
            &mut self.registry_state
        }

        registry_handlers![OutputState];
    }

    delegate_compositor!(WallpaperState);
    delegate_output!(WallpaperState);
    delegate_layer!(WallpaperState);
    delegate_registry!(WallpaperState);

    struct WaylandSurfaceHandle {
        display: *mut c_void,
        surface: *mut c_void,
    }

    impl WaylandSurfaceHandle {
        fn new(conn: &Connection, layer_surface: &LayerSurface) -> Self {
            let display = conn.backend().display_ptr() as *mut c_void;
            let surface = layer_surface.wl_surface().id().as_ptr() as *mut c_void;
            Self { display, surface }
        }
    }

    impl HasDisplayHandle for WaylandSurfaceHandle {
        fn display_handle(&self) -> StdResult<DisplayHandle<'_>, HandleError> {
            let display = NonNull::new(self.display).ok_or(HandleError::Unavailable)?;
            let wayland = WaylandDisplayHandle::new(display);
            let raw = RawDisplayHandle::Wayland(wayland);
            Ok(unsafe { DisplayHandle::borrow_raw(raw) })
        }
    }

    impl HasWindowHandle for WaylandSurfaceHandle {
        fn window_handle(&self) -> StdResult<WindowHandle<'_>, HandleError> {
            let surface = NonNull::new(self.surface).ok_or(HandleError::Unavailable)?;
            let wayland = WaylandWindowHandle::new(surface);
            let raw = RawWindowHandle::Wayland(wayland);
            Ok(unsafe { WindowHandle::borrow_raw(raw) })
        }
    }

    fn output_info_physical_size(info: OutputInfo) -> Option<PhysicalSize<u32>> {
        if let Some(mode) = info.modes.iter().find(|mode| mode.current) {
            let width = mode.dimensions.0.max(1) as u32;
            let height = mode.dimensions.1.max(1) as u32;
            return Some(PhysicalSize::new(width, height));
        }

        if let Some((width, height)) = info.logical_size {
            let scale = info.scale_factor.max(1) as u32;
            let logical_width = width.max(1) as u32;
            let logical_height = height.max(1) as u32;
            return Some(PhysicalSize::new(
                logical_width * scale,
                logical_height * scale,
            ));
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{align_of, size_of};

    /// Sanity-checks that the CPU mirror of the uniform block matches the
    /// layout baked into the GLSL header.
    #[test]
    fn shadertoy_uniforms_follow_std140_layout() {
        let uniforms = ShadertoyUniforms::new(1920, 1080);
        let base = &uniforms as *const _ as usize;

        assert_eq!(align_of::<ShadertoyUniforms>(), 16);
        assert_eq!(size_of::<ShadertoyUniforms>(), 224);
        assert_eq!((&uniforms.i_resolution as *const _ as usize) - base, 0);
        assert_eq!((&uniforms.i_time as *const _ as usize) - base, 16);
        assert_eq!((&uniforms.i_mouse as *const _ as usize) - base, 32);
        assert_eq!((&uniforms.i_date as *const _ as usize) - base, 48);
        assert_eq!((&uniforms.i_sample_rate as *const _ as usize) - base, 64);
        assert_eq!((&uniforms._padding1 as *const _ as usize) - base, 80);
        assert_eq!((&uniforms.i_channel_time as *const _ as usize) - base, 96);
        assert_eq!(
            (&uniforms.i_channel_resolution as *const _ as usize) - base,
            160
        );
    }
}

/// Wraps the user shader with our ShaderToy prelude and compiles it as GLSL.
///
/// The wrapped source is dumped to `/tmp/shaderpaper_wrapped.frag` to aid
/// debugging. Any compilation error bubbles up to the caller for logging.
fn compile_fragment_shader(device: &wgpu::Device, source: &str) -> Result<wgpu::ShaderModule> {
    let wrapped = wrap_shadertoy_fragment(source);

    if let Err(err) = std::fs::write("/tmp/shaderpaper_wrapped.frag", &wrapped) {
        eprintln!("[shaderpaper] failed to dump wrapped shader: {err}");
    }

    Ok(device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("shaderpaper fragment"),
        source: wgpu::ShaderSource::Glsl {
            shader: Cow::Owned(wrapped),
            stage: ShaderStage::Fragment,
            defines: &[],
        },
    }))
}

/// Produces a self-contained GLSL fragment shader from raw ShaderToy code.
///
/// Steps performed:
///
/// 1. Strip `#version` directives and ShaderToy uniform declarations so we can
///    inject our own definitions.
/// 2. Prepend [`HEADER`] which declares the uniform block, sampler bindings, and
///    macro aliases.
/// 3. Append [`FOOTER`] which remaps `gl_FragCoord`, calls `mainImage`, and
///    writes to `outColor`.
fn wrap_shadertoy_fragment(source: &str) -> String {
    let mut sanitized = String::new();
    let mut skipped_version = false;
    let mut sanitized_lines = Vec::new();
    for line in source.lines() {
        if !skipped_version && line.trim_start().starts_with("#version") {
            skipped_version = true;
            continue;
        }
        let trimmed = line.trim_start();
        let should_skip_uniform = trimmed.starts_with("uniform ")
            && (trimmed.contains("iResolution")
                || trimmed.contains("iTimeDelta")
                || trimmed.contains("iTime")
                || trimmed.contains("iFrame")
                || trimmed.contains("iMouse")
                || trimmed.contains("iDate")
                || trimmed.contains("iSampleRate")
                || trimmed.contains("iChannelTime")
                || trimmed.contains("iChannelResolution")
                || trimmed.contains("iChannel0")
                || trimmed.contains("iChannel1")
                || trimmed.contains("iChannel2")
                || trimmed.contains("iChannel3"));
        if should_skip_uniform {
            continue;
        }
        sanitized_lines.push(line);
    }

    for line in sanitized_lines {
        sanitized.push_str(line);
        sanitized.push('\n');
    }

    format!(
        "{HEADER}\n#line 1\n{sanitized}{FOOTER}",
        sanitized = sanitized
    )
}

/// GLSL prologue injected ahead of every ShaderToy fragment shader.
///
/// The uniform block layout must match [`ShadertoyUniforms`]. Note that we keep
/// `_iResolution` as a vec3 but reserve the fourth float for the mirrored
/// `iTime`, ensuring the shader can animate even if vec3 padding is collapsed.
const HEADER: &str = r"#version 450
layout(location = 0) in vec2 v_uv;
layout(location = 0) out vec4 outColor;

layout(std140, set = 0, binding = 0) uniform ShaderParams {
    vec3 _iResolution;
    float _iTime;
    float _iTimeDelta;
    int _iFrame;
    float _padding0;
    vec4 _iMouse;
    vec4 _iDate;
    float _iSampleRate;
    vec3 _padding1;
    float _iChannelTime[4];
    vec3 _iChannelResolution[4];
} ubo;

// Map ShaderToy names to our UBO fields via macros to avoid name clashes.
#define iResolution ubo._iResolution
#define iTime ubo._iTime
#define iTimeDelta ubo._iTimeDelta
#define iFrame ubo._iFrame
#define iMouse ubo._iMouse
#define iDate ubo._iDate
#define iSampleRate ubo._iSampleRate
#define iChannelTime ubo._iChannelTime
#define iChannelResolution ubo._iChannelResolution

layout(set = 1, binding = 0) uniform texture2D shaderpaper_channel0_texture;
layout(set = 1, binding = 1) uniform sampler shaderpaper_channel0_sampler;
layout(set = 1, binding = 2) uniform texture2D shaderpaper_channel1_texture;
layout(set = 1, binding = 3) uniform sampler shaderpaper_channel1_sampler;
layout(set = 1, binding = 4) uniform texture2D shaderpaper_channel2_texture;
layout(set = 1, binding = 5) uniform sampler shaderpaper_channel2_sampler;
layout(set = 1, binding = 6) uniform texture2D shaderpaper_channel3_texture;
layout(set = 1, binding = 7) uniform sampler shaderpaper_channel3_sampler;

#define iChannel0 sampler2D(shaderpaper_channel0_texture, shaderpaper_channel0_sampler)
#define iChannel1 sampler2D(shaderpaper_channel1_texture, shaderpaper_channel1_sampler)
#define iChannel2 sampler2D(shaderpaper_channel2_texture, shaderpaper_channel2_sampler)
#define iChannel3 sampler2D(shaderpaper_channel3_texture, shaderpaper_channel3_sampler)

vec4 shaderpaper_gl_FragCoord;
#define gl_FragCoord shaderpaper_gl_FragCoord
";

/// GLSL epilogue that remaps coordinates and delegates to `mainImage`.
const FOOTER: &str = r"void main() {
    // Capture the real builtin gl_FragCoord, then remap to ShaderToy's bottom-left origin.
    // We temporarily undef the macro so we can read the hardware builtin.
    #undef gl_FragCoord
    vec2 builtinFC = vec2(gl_FragCoord.x, gl_FragCoord.y);
    #define gl_FragCoord shaderpaper_gl_FragCoord

    vec2 fragCoord = vec2(builtinFC.x, iResolution.y - builtinFC.y);
    shaderpaper_gl_FragCoord = vec4(fragCoord, 0.0, 1.0);

    vec4 color = vec4(0.0);
    mainImage(color, fragCoord);
    outColor = color;
}
";

/// Minimal full-screen triangle vertex shader.
const VERTEX_SHADER_GLSL: &str = r"#version 450
layout(location = 0) out vec2 v_uv;

const vec2 positions[3] = vec2[3](
    vec2(-1.0, -3.0),
    vec2(3.0, 1.0),
    vec2(-1.0, 1.0)
);

void main() {
    uint vertex_index = uint(gl_VertexIndex);
    vec2 pos = positions[vertex_index];
    v_uv = pos * 0.5 + vec2(0.5, 0.5);
    gl_Position = vec4(pos, 0.0, 1.0);
}
";
