//! GPU orchestration: device/surface setup, pipelines, uniforms, channels, and rendering.
//!
//! `GpuState` encapsulates everything `wgpu`-related: it owns the instance, device,
//! queue, surface configuration, shader modules, bind group layouts, the uniform
//! buffer, and the active shader pipelines. Higher layers (`window` and `wallpaper`)
//! feed it input state (mouse/keyboard), time samples, and swap requests; `GpuState`
//! turns those into draws and optional file exports.
//!
//! High-level shape
//!
//! ```text
//!                  types::RendererConfig
//!                 /        |        \               runtime::TimeSample
//!           channels    color/alpha   AA                    │
//!                │         │          │                    ▼
//!  bindings → layout → surface format → MSAA → GpuState ← uniforms (UBO)
//!                              │                 │
//!         compile::{vertex, fragment}            │ mouse/keyboard
//!                              │                 │
//!                              ▼                 ▼
//!                     ShaderPipeline(s) ──▶ render pass ──▶ surface frame
//!                           ▲       ▲
//!                           │       └── crossfade(previous ⇄ current)
//!                           └── pending (warmup) → promote → previous
//! ```
//!
//! Pipelines and transitions
//!
//! - `current` is the live pipeline; `previous` is kept during a crossfade.
//! - `pending` is built in the background (warmup frames rendered at 0% mix) and
//!   promoted to `current` with an optional crossfade. This ensures a smooth swap
//!   without a hitch when changing shaders.
//!
//! Uniforms and ShaderToy mapping
//!
//! - The std140 `ShadertoyUniforms` block mirrors macros injected by `compile::HEADER`.
//!   We update it per-frame and copy via a staging buffer inside the encoder to
//!   ensure each render pass is coherent.
//! - Channel resolutions are written from materialised resources before drawing.
//!
//! Channels
//!
//! - For each bound channel (`iChannel0..3`) we create textures/samplers based on the
//!   `ChannelTextureKind` signature (`Texture2d` or `Cubemap`).
//! - A special keyboard channel is a 256×3 RGBA texture: rows encode state/pulse/toggle.
//!   `window` keeps that texture up to date via `update_keyboard_channels`.
//!
//! Color, MSAA, and formats
//!
//! - `ColorSpaceMode::{Gamma,Linear}` chooses non-sRGB vs sRGB surface formats.
//! - MSAA sample count is resolved against adapter/format capabilities and clamped for
//!   stability (notably on software rasterizers or without adapter-specific features).
//!
//! GPU resource friendliness
//!
//! - Adapter request honours `GpuPowerPreference` (default LowPower) to yield priority.
//! - Device creation uses `GpuMemoryMode` hints (default Balanced) to reduce pressure.
//! - Swapchain uses a configurable `gpu_latency` (default 2) for better scheduling
//!   alongside foreground apps.
//!
//! Rendering paths
//!
//! - `render` presents into the surface; `render_export` reads back to PNG/EXR if the
//!   surface supports COPY_SRC, otherwise it presents-only and reports success.
//!
//! Primary entry points
//!
//! - `GpuState::new` — build surface/device, layouts, and the initial pipeline.
//! - `render` / `render_export` — draw a frame, optionally capture.
//! - `set_shader` — compile a new fragment shader and schedule warmup/crossfade.
//! - `resize` — reconfigure the surface and MSAA target.
//! - `update_keyboard_channels` — refresh the keyboard texture when present.
//!
use std::fmt;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use bytemuck::{Pod, Zeroable};
use chrono::{Datelike, Local, Timelike};
use crossbeam_channel::{unbounded, Receiver, Sender};
use image::imageops::flip_vertical_in_place;
use image::{codecs::png::PngEncoder, ExtendedColorType, ImageEncoder};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use wgpu::util::{DeviceExt, TextureDataOrder};
use wgpu::TextureFormatFeatureFlags;
use winit::dpi::PhysicalSize;

use crate::compile::{compile_fragment_shader, compile_vertex_shader};
use crate::runtime::{ExportFormat, FillMethod, TimeSample};
use crate::types::{
    AdapterProfile, Antialiasing, ChannelBindings, ChannelSource, ChannelTextureKind,
    ColorSpaceMode, GpuMemoryMode, GpuPowerPreference, ShaderCompiler, CHANNEL_COUNT,
    CUBEMAP_FACE_STEMS,
};

const KEYBOARD_TEXTURE_WIDTH: u32 = 256;
const KEYBOARD_TEXTURE_HEIGHT: u32 = 3;
const KEYBOARD_BYTES_PER_PIXEL: u32 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResolvedColorSpace {
    Gamma,
    Linear,
}

struct PrepareJob {
    id: u64,
    shader_path: PathBuf,
    channel_bindings: ChannelBindings,
    channel_kinds: [ChannelTextureKind; CHANNEL_COUNT],
    crossfade: Duration,
    warmup: Duration,
    requested_at: Instant,
}

enum PrepareCommand {
    Prepare(Box<PrepareJob>),
    Shutdown,
}

enum PrepareResult {
    Ready {
        id: u64,
        pipeline: ShaderPipeline,
        crossfade: Duration,
        warmup: Duration,
        requested_at: Instant,
        finished_at: Instant,
        shader_path: PathBuf,
    },
    Failed {
        id: u64,
        error: anyhow::Error,
        requested_at: Instant,
        shader_path: PathBuf,
    },
}

struct PrepareWorkerContext {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline_layout: wgpu::PipelineLayout,
    channel_layout: wgpu::BindGroupLayout,
    vertex_module: wgpu::ShaderModule,
    surface_format: wgpu::TextureFormat,
    sample_count: u32,
    color_space: ResolvedColorSpace,
    shader_compiler: ShaderCompiler,
}

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
    channel_kinds: [ChannelTextureKind; CHANNEL_COUNT],
    uniforms: ShadertoyUniforms,
    current: ShaderPipeline,
    previous: Option<ShaderPipeline>,
    crossfade: Option<CrossfadeState>,
    pending: Option<PendingPipeline>,
    start_time: Instant,
    last_frame_time: Instant,
    frame_count: u32,
    last_override_sample: Option<TimeSample>,
    render_scale: f32,
    fill_method: FillMethod,
    adapter_profile: AdapterProfile,
    surface_supports_copy: bool,
    fps_sample_frame: u32,
    fps_sample_time: Instant,
    last_measured_fps: f32,
    prepare_tx: Sender<PrepareCommand>,
    prepare_rx: Receiver<PrepareResult>,
    prepare_thread: Option<thread::JoinHandle<()>>,
    next_prepare_id: u64,
    pending_request_id: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct FileExportTarget {
    pub path: PathBuf,
    pub format: ExportFormat,
}

#[derive(Debug)]
pub enum RenderExportError {
    Surface(wgpu::SurfaceError),
    Export(anyhow::Error),
}

impl RenderExportError {
    pub fn as_surface_error(&self) -> Option<&wgpu::SurfaceError> {
        match self {
            RenderExportError::Surface(err) => Some(err),
            _ => None,
        }
    }
}

impl From<wgpu::SurfaceError> for RenderExportError {
    fn from(value: wgpu::SurfaceError) -> Self {
        RenderExportError::Surface(value)
    }
}

impl fmt::Display for RenderExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RenderExportError::Surface(err) => write!(f, "surface error: {err:?}"),
            RenderExportError::Export(err) => write!(f, "export failed: {err}"),
        }
    }
}

impl std::error::Error for RenderExportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            RenderExportError::Surface(err) => Some(err),
            RenderExportError::Export(err) => Some(err.as_ref()),
        }
    }
}

impl GpuState {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new<T>(
        target: &T,
        initial_size: PhysicalSize<u32>,
        shader_source: &Path,
        channel_bindings: &ChannelBindings,
        antialiasing: Antialiasing,
        color_space: ColorSpaceMode,
        shader_compiler: ShaderCompiler,
        render_scale: f32,
        fill_method: FillMethod,
        gpu_power: GpuPowerPreference,
        gpu_memory: GpuMemoryMode,
        gpu_latency: u32,
    ) -> Result<Self>
    where
        T: HasDisplayHandle + HasWindowHandle,
    {
        // Note: Some systems may emit EGL fence sync errors like:
        // "EGL 'eglCreateSyncKHR' code 0x3004: EGL_BAD_ATTRIBUTE error: In eglCreateSyncKHR:
        //  EGL_SYNC_NATIVE_FENCE_FD_ANDROID specified valid fd butEGL_SYNC_STATUS is also being set"
        // This appears to be a driver/EGL implementation issue with fence synchronization
        // and does not affect rendering functionality.
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            flags: wgpu::InstanceFlags::default(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            backend_options: wgpu::BackendOptions::default(),
        });
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

        // Use LowPower preference to be friendly to other GPU applications.
        // Wallpaper rendering doesn't need maximum GPU priority and should yield
        // to interactive applications like browsers.
        let power_preference = match gpu_power {
            GpuPowerPreference::Low => wgpu::PowerPreference::LowPower,
            GpuPowerPreference::High => wgpu::PowerPreference::HighPerformance,
        };
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .context("failed to find a suitable GPU adapter")?;

        let adapter_info = adapter.get_info();
        let limits = adapter.limits();
        let adapter_profile = AdapterProfile::from_wgpu(&adapter_info, &limits);
        let is_software = adapter_profile.is_software();
        tracing::debug!(
            name = %adapter_profile.name,
            backend = ?adapter_profile.backend,
            device_type = ?adapter_profile.device_type,
            is_software,
            "selected GPU adapter"
        );

        let adapter_features = adapter.features();
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
        let resolved_color = match color_space {
            ColorSpaceMode::Auto | ColorSpaceMode::Gamma => ResolvedColorSpace::Gamma,
            ColorSpaceMode::Linear => ResolvedColorSpace::Linear,
        };

        let surface_format = match resolved_color {
            ResolvedColorSpace::Linear => surface_caps
                .formats
                .iter()
                .copied()
                .find(|format| format.is_srgb())
                .unwrap_or_else(|| {
                    let fallback = surface_caps.formats[0];
                    if !fallback.is_srgb() {
                        tracing::warn!(
                            ?fallback,
                            "no sRGB surface format available; falling back to {:?}",
                            fallback
                        );
                    }
                    fallback
                }),
            ResolvedColorSpace::Gamma => surface_caps
                .formats
                .iter()
                .copied()
                .find(|format| !format.is_srgb())
                .unwrap_or_else(|| {
                    let fallback = surface_caps.formats[0];
                    if fallback.is_srgb() {
                        tracing::warn!(
                            ?fallback,
                            "no linear (non-sRGB) surface format available; falling back to {:?}",
                            fallback
                        );
                    }
                    fallback
                }),
        };

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

        if is_software && sample_count > 1 {
            tracing::warn!(
                sample_count,
                "software rasterizer detected; disabling MSAA for performance"
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

        tracing::debug!(
            ?antialiasing,
            sample_count,
            supported_samples = ?supported_samples,
            "resolved MSAA configuration"
        );

        let mut required_features = wgpu::Features::empty();
        if sample_count > 4 {
            required_features |= wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES;
        }

        // Use MemoryUsage instead of Performance to reduce GPU memory pressure.
        // This allows other applications to allocate GPU resources more easily.
        let memory_hints = match gpu_memory {
            GpuMemoryMode::Balanced => wgpu::MemoryHints::MemoryUsage,
            GpuMemoryMode::Performance => wgpu::MemoryHints::Performance,
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("wax11 device"),
            required_features,
            required_limits: limits.clone(),
            memory_hints,
            trace: wgpu::Trace::default(),
        }))
        .context("failed to create GPU device")?;

        let size = PhysicalSize::new(requested_width, requested_height);
        let surface_supports_copy = surface_caps.usages.contains(wgpu::TextureUsages::COPY_SRC);
        let mut surface_usage = wgpu::TextureUsages::RENDER_ATTACHMENT;
        if surface_supports_copy {
            surface_usage |= wgpu::TextureUsages::COPY_SRC;
        } else {
            tracing::warn!(
                "surface does not advertise COPY_SRC; still-export will fall back to presenting only"
            );
        }

        // Prefer FIFO present mode for maximum stability on Wayland/NVIDIA.
        // This avoids driver quirks seen with other modes on some stacks.
        let present_mode = surface_caps
            .present_modes
            .iter()
            .copied()
            .find(|m| *m == wgpu::PresentMode::Fifo)
            .unwrap_or_else(|| surface_caps.present_modes[0]);
        tracing::debug!(?present_mode, "using present mode");

        // Use frame latency of 2 instead of 1 to reduce GPU contention with other applications.
        // Wallpaper rendering doesn't require minimal latency, and this allows the driver to
        // better schedule work alongside interactive applications like browsers.
        let frame_latency = gpu_latency.clamp(1, 3);
        if frame_latency != gpu_latency {
            tracing::warn!(
                requested = gpu_latency,
                clamped = frame_latency,
                "GPU frame latency clamped to valid range (1-3)"
            );
        }
        let config = wgpu::SurfaceConfiguration {
            usage: surface_usage,
            format: surface_format,
            width: size.width,
            height: size.height,
            present_mode,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: frame_latency,
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

        let channel_kinds = channel_bindings.layout_signature();

        let channel_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("channel layout"),
            entries: &build_channel_layout_entries(&channel_kinds),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shader pipeline layout"),
            bind_group_layouts: &[&uniform_layout, &channel_layout],
            push_constant_ranges: &[],
        });

        let vertex_module = compile_vertex_shader(&device, shader_compiler)?;

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
            &channel_kinds,
            resolved_color,
            shader_compiler,
        )?;

        let uniforms = ShadertoyUniforms::new(size.width, size.height);

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

        let worker_context = PrepareWorkerContext {
            device: device.clone(),
            queue: queue.clone(),
            pipeline_layout: pipeline_layout.clone(),
            channel_layout: channel_layout.clone(),
            vertex_module: vertex_module.clone(),
            surface_format,
            sample_count,
            color_space: resolved_color,
            shader_compiler,
        };

        let (prepare_tx, prepare_rx, prepare_thread) = spawn_prepare_worker(worker_context)?;

        let mut state = Self {
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
            channel_kinds,
            uniforms,
            current,
            previous: None,
            crossfade: None,
            pending: None,
            start_time: Instant::now(),
            last_frame_time: Instant::now(),
            frame_count: 0,
            last_override_sample: None,
            render_scale,
            fill_method,
            adapter_profile,
            surface_supports_copy,
            fps_sample_frame: 0,
            fps_sample_time: Instant::now(),
            last_measured_fps: 60.0,
            prepare_tx,
            prepare_rx,
            prepare_thread: Some(prepare_thread),
            next_prepare_id: 1,
            pending_request_id: None,
        };
        state.recompute_view_uniforms();
        state.queue.write_buffer(
            &state.uniform_buffer,
            0,
            bytemuck::bytes_of(&state.uniforms),
        );
        Ok(state)
    }

    pub(crate) fn size(&self) -> PhysicalSize<u32> {
        self.size
    }

    pub(crate) fn channel_kinds(&self) -> &[ChannelTextureKind; CHANNEL_COUNT] {
        &self.channel_kinds
    }

    pub(crate) fn adapter_profile(&self) -> &AdapterProfile {
        &self.adapter_profile
    }

    pub(crate) fn has_keyboard_channel(&self) -> bool {
        self.current.has_keyboard_channel()
            || self
                .pending
                .as_ref()
                .map(|pending| pending.pipeline.has_keyboard_channel())
                .unwrap_or(false)
    }

    pub(crate) fn update_keyboard_channels(&self, data: &[u8]) {
        let queue = &self.queue;
        self.current.update_keyboard_channels(queue, data);
        if let Some(previous) = &self.previous {
            previous.update_keyboard_channels(queue, data);
        }
        if let Some(pending) = &self.pending {
            pending.pipeline.update_keyboard_channels(queue, data);
        }
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
        self.recompute_view_uniforms();
    }

    pub(crate) fn set_shader(
        &mut self,
        shader_source: &Path,
        channel_bindings: &ChannelBindings,
        crossfade: Duration,
        warmup: Duration,
        now: Instant,
    ) -> Result<()> {
        debug_assert_eq!(
            &self.channel_kinds,
            &channel_bindings.layout_signature(),
            "channel layout mismatch; caller must rebuild GPU state"
        );
        self.pending = None;
        let request_id = self.next_prepare_id;
        self.next_prepare_id = self.next_prepare_id.wrapping_add(1);
        self.pending_request_id = Some(request_id);
        let job = PrepareJob {
            id: request_id,
            shader_path: shader_source.to_path_buf(),
            channel_bindings: channel_bindings.clone(),
            channel_kinds: self.channel_kinds,
            crossfade,
            warmup,
            requested_at: now,
        };
        self.prepare_tx
            .send(PrepareCommand::Prepare(Box::new(job)))
            .map_err(|err| anyhow!("failed to dispatch shader prepare job: {err}"))?;
        tracing::debug!(
            shader = %shader_source.display(),
            crossfade_ms = crossfade.as_millis(),
            warmup_ms = warmup.as_millis(),
            "queued asynchronous shader preparation"
        );

        Ok(())
    }

    pub(crate) fn render(
        &mut self,
        mouse: [f32; 4],
        time_sample: Option<TimeSample>,
    ) -> Result<(), wgpu::SurfaceError> {
        let frame = self.render_internal(mouse, time_sample, |_, _| {})?;
        frame.present();
        Ok(())
    }

    pub(crate) fn render_export(
        &mut self,
        mouse: [f32; 4],
        time_sample: Option<TimeSample>,
        target: &FileExportTarget,
    ) -> Result<PathBuf, RenderExportError> {
        if !self.surface_supports_copy {
            return Err(RenderExportError::Export(anyhow!(
                "surface does not support COPY_SRC; cannot export still frame on this backend"
            )));
        }
        let capture = FrameCapture::new(&self.device, self.config.format, self.size)
            .map_err(RenderExportError::Export)?;
        let frame = self
            .render_internal(mouse, time_sample, |surface, encoder| {
                capture.encode_copy(&surface.texture, encoder);
            })
            .map_err(RenderExportError::Surface)?;
        frame.present();
        let resolved = capture
            .resolve(&self.device)
            .map_err(RenderExportError::Export)?;
        target.write(&resolved).map_err(RenderExportError::Export)?;
        tracing::info!(
            path = %target.path.display(),
            width = resolved.width,
            height = resolved.height,
            "exported still frame"
        );
        Ok(target.path.clone())
    }

    fn render_internal<F>(
        &mut self,
        mouse: [f32; 4],
        time_sample: Option<TimeSample>,
        mut with_encoder: F,
    ) -> Result<wgpu::SurfaceTexture, wgpu::SurfaceError>
    where
        F: FnMut(&wgpu::SurfaceTexture, &mut wgpu::CommandEncoder),
    {
        let now = Instant::now();
        self.update_time(mouse, now, time_sample);
        self.poll_prepare_results(now);

        let mut warmup_state = None;
        if let Some(pending) = self.pending.take() {
            if now >= pending.warmup_end {
                self.begin_crossfade(pending.pipeline, pending.crossfade, now);
            } else {
                warmup_state = Some(pending);
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

        if let Some((prev_mix, curr_mix)) = self.crossfade.as_ref().map(CrossfadeState::mixes) {
            if prev_mix > f32::EPSILON {
                if let Some(prev) = previous_pipeline.as_ref() {
                    self.render_with_pipeline(&mut encoder, &view, prev, prev_mix, load);
                    load = wgpu::LoadOp::Load;
                }
            }

            if curr_mix > f32::EPSILON {
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

            // Advance the crossfade and check if it's finished
            let finished = if let Some(fade) = self.crossfade.as_mut() {
                fade.advance();
                fade.is_finished()
            } else {
                false
            };

            // Clean up when crossfade completes
            if finished {
                previous_pipeline = None;
                self.crossfade = None;
                tracing::debug!("crossfade completed, cleaned up previous pipeline");
            }
        } else {
            // No active crossfade, ensure previous is cleared
            previous_pipeline = None;
            unsafe {
                self.render_with_pipeline(&mut encoder, &view, &*current_pipeline_ptr, 1.0, load);
            }
        }

        if let Some(pending) = warmup_state {
            self.render_with_pipeline(
                &mut encoder,
                &view,
                &pending.pipeline,
                0.0,
                wgpu::LoadOp::Load,
            );
            self.pending = Some(pending);
        }

        with_encoder(&frame, &mut encoder);

        self.previous = previous_pipeline;

        self.queue.submit(std::iter::once(encoder.finish()));

        Ok(frame)
    }

    fn poll_prepare_results(&mut self, now: Instant) {
        while let Ok(result) = self.prepare_rx.try_recv() {
            match result {
                PrepareResult::Ready {
                    id,
                    pipeline,
                    crossfade,
                    warmup,
                    requested_at,
                    finished_at,
                    shader_path,
                } => {
                    let elapsed = finished_at.saturating_duration_since(requested_at);
                    tracing::debug!(
                        shader = %shader_path.display(),
                        elapsed_ms = elapsed.as_millis(),
                        "shader preparation completed"
                    );
                    if self.pending_request_id == Some(id) {
                        self.pending_request_id = None;
                        self.pending = Some(PendingPipeline {
                            pipeline,
                            crossfade,
                            warmup_end: now + warmup,
                        });
                    } else {
                        tracing::trace!(
                            shader = %shader_path.display(),
                            id,
                            "discarded stale shader preparation result"
                        );
                    }
                }
                PrepareResult::Failed {
                    id,
                    error,
                    requested_at,
                    shader_path,
                } => {
                    let elapsed = now.saturating_duration_since(requested_at);
                    tracing::error!(
                        shader = %shader_path.display(),
                        elapsed_ms = elapsed.as_millis(),
                        error = %error,
                        "shader preparation failed"
                    );
                    if self.pending_request_id == Some(id) {
                        self.pending_request_id = None;
                    }
                }
            }
        }
    }

    fn logical_dimensions(&self) -> (f32, f32) {
        let surface_w = self.size.width.max(1) as f32;
        let surface_h = self.size.height.max(1) as f32;
        let scale = self.render_scale.max(0.0001);
        match self.fill_method {
            FillMethod::Stretch | FillMethod::Tile { .. } => (surface_w * scale, surface_h * scale),
            FillMethod::Center {
                content_width,
                content_height,
            } => (
                (content_width as f32).max(1.0) * scale,
                (content_height as f32).max(1.0) * scale,
            ),
        }
    }

    fn recompute_view_uniforms(&mut self) {
        let surface_w = self.size.width.max(1) as f32;
        let surface_h = self.size.height.max(1) as f32;
        let (logical_w, logical_h) = self.logical_dimensions();

        let mut scale_x = if surface_w > 0.0 {
            logical_w / surface_w
        } else {
            self.render_scale.max(0.0001)
        };
        let mut scale_y = if surface_h > 0.0 {
            logical_h / surface_h
        } else {
            self.render_scale.max(0.0001)
        };
        let mut offset_x = 0.0_f32;
        let mut offset_y = 0.0_f32;
        let mut wrap_x = 0.0_f32;
        let mut wrap_y = 0.0_f32;

        match self.fill_method {
            FillMethod::Stretch => {}
            FillMethod::Center {
                content_width,
                content_height,
            } => {
                let scale = self.render_scale.max(0.0001);
                let content_w = (content_width as f32).max(1.0);
                let content_h = (content_height as f32).max(1.0);
                let content_physical_w = content_w.min(surface_w);
                let content_physical_h = content_h.min(surface_h);

                if content_physical_w > 0.0 {
                    scale_x = (content_w * scale) / content_physical_w;
                }
                if content_physical_h > 0.0 {
                    scale_y = (content_h * scale) / content_physical_h;
                }

                let left = (surface_w - content_physical_w) * 0.5;
                let bottom = (surface_h - content_physical_h) * 0.5;
                offset_x = -left * scale_x;
                offset_y = -bottom * scale_y;
            }
            FillMethod::Tile { repeat_x, repeat_y } => {
                let repeats_x = repeat_x.max(0.0);
                let repeats_y = repeat_y.max(0.0);
                if repeats_x > 0.0 {
                    wrap_x = logical_w / repeats_x;
                    scale_x *= repeats_x;
                }
                if repeats_y > 0.0 {
                    wrap_y = logical_h / repeats_y;
                    scale_y *= repeats_y;
                }
            }
        }

        self.uniforms.set_resolution(logical_w, logical_h);
        self.uniforms
            .set_surface(surface_w, surface_h, logical_w, logical_h);
        self.uniforms.set_fill(scale_x, scale_y, offset_x, offset_y);
        self.uniforms.set_fill_wrap(wrap_x, wrap_y);
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
        self.recompute_view_uniforms();

        // Write uniforms to a staging buffer, then copy to the uniform buffer within the encoder.
        // This ensures each render pass sees its own uniform values.
        let staging = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("uniform staging"),
                contents: bytemuck::bytes_of(&self.uniforms),
                usage: wgpu::BufferUsages::COPY_SRC,
            });
        encoder.copy_buffer_to_buffer(
            &staging,
            0,
            &self.uniform_buffer,
            0,
            std::mem::size_of::<ShadertoyUniforms>() as u64,
        );

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

    fn update_time(&mut self, mouse: [f32; 4], now: Instant, override_sample: Option<TimeSample>) {
        let (seconds, delta_seconds, frame_index, next_frame_count) =
            if let Some(sample) = override_sample {
                let previous = self.last_override_sample.replace(sample);
                let delta = previous
                    .map(|prev| (sample.seconds - prev.seconds).max(0.0))
                    .unwrap_or(0.0);

                self.start_time = now;
                self.last_frame_time = now;
                let frame_value = sample.frame_index.min(i32::MAX as u64) as i32;
                let next_count = sample
                    .frame_index
                    .saturating_add(1)
                    .min(u64::from(u32::MAX)) as u32;
                (sample.seconds, delta, frame_value, next_count)
            } else {
                self.last_override_sample = None;
                if self.frame_count == 0 {
                    self.start_time = now;
                    self.last_frame_time = now;
                }
                let elapsed = now.duration_since(self.start_time);
                let delta = now.duration_since(self.last_frame_time);
                self.last_frame_time = now;
                let frame_index = self.frame_count as i32;
                let next_count = self.frame_count.saturating_add(1);
                (
                    elapsed.as_secs_f32(),
                    delta.as_secs_f32(),
                    frame_index,
                    next_count,
                )
            };

        self.uniforms.i_time = seconds;
        self.uniforms.i_time_delta = delta_seconds;
        self.uniforms.i_frame = frame_index;
        self.frame_count = next_frame_count;
        for channel in &mut self.uniforms.i_channel_time {
            channel[0] = self.uniforms.i_time;
        }
        self.uniforms.i_resolution[3] = self.uniforms.i_time;
        self.uniforms.i_mouse = mouse;

        let fps_elapsed = now.saturating_duration_since(self.fps_sample_time);
        if fps_elapsed >= Duration::from_secs(1) {
            let frames_since = self.frame_count.saturating_sub(self.fps_sample_frame);
            if frames_since > 0 && fps_elapsed.as_secs_f32() > f32::EPSILON {
                self.last_measured_fps = frames_since as f32 / fps_elapsed.as_secs_f32();
            }
            self.fps_sample_time = now;
            self.fps_sample_frame = self.frame_count;
        }

        let local_now = Local::now();
        let seconds_since_midnight = local_now.num_seconds_from_midnight() as f32
            + local_now.nanosecond() as f32 / 1_000_000_000.0;
        self.uniforms.i_date = [
            local_now.year() as f32,
            local_now.month() as f32,
            local_now.day() as f32,
            seconds_since_midnight,
        ];
    }

    fn begin_crossfade(&mut self, pipeline: ShaderPipeline, crossfade: Duration, _now: Instant) {
        // TODO: allow this to be dynamic based on framerate
        // 16ms ~ 1/60 a second, below that is a hard-cut
        let crossfade = if crossfade < Duration::from_millis(16) {
            Duration::ZERO
        } else {
            crossfade
        };

        if crossfade.is_zero() {
            tracing::debug!("hard-cut shader swap (zero crossfade)");
            self.current = pipeline;
            self.previous = None;
            self.crossfade = None;
        } else {
            let previous = std::mem::replace(&mut self.current, pipeline);
            self.previous = Some(previous);
            let duration_secs = crossfade.as_secs_f32().max(f32::EPSILON);
            let fps = self.last_measured_fps.max(1.0);
            let steps = (duration_secs * fps).ceil().max(1.0) as u32;
            self.crossfade = Some(CrossfadeState::new(steps));
            tracing::debug!(
                crossfade_ms = crossfade.as_millis(),
                fps = %fps,
                steps = steps,
                "starting crossfade transition"
            );
        }
    }
}

impl Drop for GpuState {
    fn drop(&mut self) {
        let _ = self.prepare_tx.send(PrepareCommand::Shutdown);
        if let Some(handle) = self.prepare_thread.take() {
            let _ = handle.join();
        }
    }
}

struct ShaderPipeline {
    pipeline: wgpu::RenderPipeline,
    channel_bind_group: wgpu::BindGroup,
    channel_resources: Vec<ChannelResources>,
}

impl ShaderPipeline {
    #[allow(clippy::too_many_arguments)]
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
        channel_kinds: &[ChannelTextureKind; CHANNEL_COUNT],
        color_space: ResolvedColorSpace,
        shader_compiler: ShaderCompiler,
    ) -> Result<Self> {
        let shader_code = std::fs::read_to_string(shader_path)
            .with_context(|| format!("failed to read shader at {}", shader_path.display()))?;
        let fragment_module = compile_fragment_shader(device, &shader_code, shader_compiler)
            .context("failed to compile shader")?;

        let channel_resources = create_channel_resources(
            device,
            queue,
            channel_bindings.slots(),
            channel_kinds,
            color_space,
        )?;
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
            pipeline,
            channel_bind_group,
            channel_resources,
        })
    }

    fn has_keyboard_channel(&self) -> bool {
        self.channel_resources
            .iter()
            .any(ChannelResources::is_keyboard)
    }

    fn update_keyboard_channels(&self, queue: &wgpu::Queue, data: &[u8]) {
        if !self.has_keyboard_channel() {
            return;
        }
        for resource in &self.channel_resources {
            resource.update_keyboard(queue, data);
        }
    }
}

struct CrossfadeState {
    steps: u32,
    current_step: u32,
    step_size: f32,
    progress: f32,
}

impl CrossfadeState {
    fn new(steps: u32) -> Self {
        let steps = steps.max(1);
        let step_size = 1.0 / steps as f32;
        Self {
            steps,
            current_step: 0,
            step_size,
            progress: 0.0,
        }
    }

    fn mixes(&self) -> (f32, f32) {
        (1.0 - self.progress, self.progress)
    }

    fn advance(&mut self) {
        if self.current_step < self.steps {
            self.current_step += 1;
            let next = self.current_step as f32 * self.step_size;
            self.progress = next.min(1.0);
        }
    }

    fn is_finished(&self) -> bool {
        self.progress >= 1.0
    }
}

struct PendingPipeline {
    pipeline: ShaderPipeline,
    crossfade: Duration,
    warmup_end: Instant,
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
    i_surface: [f32; 4],
    i_fill: [f32; 4],
    i_fill_wrap: [f32; 4],
}

unsafe impl Zeroable for ShadertoyUniforms {}
unsafe impl Pod for ShadertoyUniforms {}

impl ShadertoyUniforms {
    fn new(width: u32, height: u32) -> Self {
        let mut uniforms = Self {
            i_resolution: [width as f32, height as f32, 0.0, 0.0],
            i_surface: [width as f32, height as f32, width as f32, height as f32],
            i_fill: [1.0, 1.0, 0.0, 0.0],
            i_fill_wrap: [0.0, 0.0, 0.0, 0.0],
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

    fn set_surface(&mut self, surface_w: f32, surface_h: f32, logical_w: f32, logical_h: f32) {
        self.i_surface[0] = surface_w;
        self.i_surface[1] = surface_h;
        self.i_surface[2] = logical_w;
        self.i_surface[3] = logical_h;
    }

    fn set_fill(&mut self, scale_x: f32, scale_y: f32, offset_x: f32, offset_y: f32) {
        self.i_fill[0] = scale_x;
        self.i_fill[1] = scale_y;
        self.i_fill[2] = offset_x;
        self.i_fill[3] = offset_y;
    }

    fn set_fill_wrap(&mut self, wrap_x: f32, wrap_y: f32) {
        self.i_fill_wrap[0] = wrap_x;
        self.i_fill_wrap[1] = wrap_y;
    }
}

fn build_channel_layout_entries(
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

struct KeyboardTextureMeta {
    width: u32,
    height: u32,
}

struct ChannelResources {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    sampler: wgpu::Sampler,
    resolution: [f32; 4],
    keyboard: Option<KeyboardTextureMeta>,
}

impl ChannelResources {
    fn is_keyboard(&self) -> bool {
        self.keyboard.is_some()
    }

    fn update_keyboard(&self, queue: &wgpu::Queue, data: &[u8]) {
        let Some(meta) = &self.keyboard else {
            return;
        };

        let expected_len = (meta.width * meta.height * KEYBOARD_BYTES_PER_PIXEL) as usize;
        if data.len() != expected_len {
            tracing::warn!(
                channel_width = meta.width,
                channel_height = meta.height,
                expected_len,
                actual_len = data.len(),
                "keyboard texture update ignored due to mismatched payload size"
            );
            return;
        }

        let bytes_per_row = meta.width * KEYBOARD_BYTES_PER_PIXEL;
        let layout = wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(bytes_per_row),
            rows_per_image: Some(meta.height),
        };
        let extent = wgpu::Extent3d {
            width: meta.width,
            height: meta.height,
            depth_or_array_layers: 1,
        };
        let texture_info = wgpu::TexelCopyTextureInfo {
            texture: &self.texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        };
        queue.write_texture(texture_info, data, layout, extent);
    }
}

fn spawn_prepare_worker(
    context: PrepareWorkerContext,
) -> Result<(
    Sender<PrepareCommand>,
    Receiver<PrepareResult>,
    thread::JoinHandle<()>,
)> {
    let (command_tx, command_rx) = unbounded();
    let (result_tx, result_rx) = unbounded();
    let PrepareWorkerContext {
        device,
        queue,
        pipeline_layout,
        channel_layout,
        vertex_module,
        surface_format,
        sample_count,
        color_space,
        shader_compiler,
    } = context;
    let thread = thread::Builder::new()
        .name("wax11-shader-prep".into())
        .spawn(move || {
            while let Ok(command) = command_rx.recv() {
                match command {
                    PrepareCommand::Prepare(job) => {
                        let PrepareJob {
                            id,
                            shader_path,
                            channel_bindings,
                            channel_kinds,
                            crossfade,
                            warmup,
                            requested_at,
                        } = *job;
                        let result = ShaderPipeline::new(
                            &device,
                            &queue,
                            &pipeline_layout,
                            &channel_layout,
                            &vertex_module,
                            surface_format,
                            sample_count,
                            shader_path.as_path(),
                            &channel_bindings,
                            &channel_kinds,
                            color_space,
                            shader_compiler,
                        );
                        match result {
                            Ok(pipeline) => {
                                let finished_at = Instant::now();
                                let _ = result_tx.send(PrepareResult::Ready {
                                    id,
                                    pipeline,
                                    crossfade,
                                    warmup,
                                    requested_at,
                                    finished_at,
                                    shader_path,
                                });
                            }
                            Err(error) => {
                                let _ = result_tx.send(PrepareResult::Failed {
                                    id,
                                    error,
                                    requested_at,
                                    shader_path,
                                });
                            }
                        }
                    }
                    PrepareCommand::Shutdown => break,
                }
            }
        })
        .context("failed to spawn shader preparation worker")?;
    Ok((command_tx, result_rx, thread))
}

fn create_channel_resources(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    bindings: &[Option<ChannelSource>; CHANNEL_COUNT],
    kinds: &[ChannelTextureKind; CHANNEL_COUNT],
    color_space: ResolvedColorSpace,
) -> Result<Vec<ChannelResources>> {
    let mut resources = Vec::with_capacity(CHANNEL_COUNT);
    for (index, (binding, kind)) in bindings.iter().zip(kinds.iter()).enumerate() {
        let resource = match (binding, kind) {
            (Some(ChannelSource::Texture { path }), ChannelTextureKind::Texture2d) => {
                match load_texture_channel(device, queue, index, path, color_space) {
                    Ok(resource) => resource,
                    Err(err) => {
                        tracing::warn!(
                            channel = index,
                            path = %path.display(),
                            error = %err,
                            "failed to load texture channel; using placeholder"
                        );
                        create_placeholder_texture(device, queue, index as u32, color_space)?
                    }
                }
            }
            (Some(ChannelSource::Keyboard), ChannelTextureKind::Texture2d) => {
                create_keyboard_channel(device, queue, index as u32, color_space)?
            }
            (Some(ChannelSource::Cubemap { directory }), ChannelTextureKind::Cubemap) => {
                match load_cubemap_channel(device, queue, index, directory, color_space) {
                    Ok(resource) => resource,
                    Err(err) => {
                        tracing::warn!(
                            channel = index,
                            dir = %directory.display(),
                            error = %err,
                            "failed to load cubemap channel; using placeholder"
                        );
                        create_placeholder_cubemap(device, queue, index as u32, color_space)?
                    }
                }
            }
            (None, ChannelTextureKind::Texture2d) => {
                create_placeholder_texture(device, queue, index as u32, color_space)?
            }
            (None, ChannelTextureKind::Cubemap) => {
                create_placeholder_cubemap(device, queue, index as u32, color_space)?
            }
            (Some(ChannelSource::Texture { .. }), ChannelTextureKind::Cubemap)
            | (Some(ChannelSource::Cubemap { .. }), ChannelTextureKind::Texture2d)
            | (Some(ChannelSource::Keyboard), ChannelTextureKind::Cubemap) => {
                tracing::warn!(
                    channel = index,
                    "channel binding kind mismatch; using placeholder"
                );
                match kind {
                    ChannelTextureKind::Texture2d => {
                        create_placeholder_texture(device, queue, index as u32, color_space)?
                    }
                    ChannelTextureKind::Cubemap => {
                        create_placeholder_cubemap(device, queue, index as u32, color_space)?
                    }
                }
            }
        };
        resources.push(resource);
    }

    Ok(resources)
}

fn create_placeholder_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    index: u32,
    color_space: ResolvedColorSpace,
) -> Result<ChannelResources> {
    let data = [255u8, 255, 255, 255];
    let texture_format = match color_space {
        ResolvedColorSpace::Gamma => wgpu::TextureFormat::Rgba8Unorm,
        ResolvedColorSpace::Linear => wgpu::TextureFormat::Rgba8UnormSrgb,
    };
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
            format: texture_format,
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
        texture,
        view,
        sampler,
        resolution: [1.0, 1.0, 1.0, 0.0],
        keyboard: None,
    })
}

fn create_placeholder_cubemap(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    index: u32,
    color_space: ResolvedColorSpace,
) -> Result<ChannelResources> {
    let data = vec![255u8; 4 * 6];
    let texture_format = match color_space {
        ResolvedColorSpace::Gamma => wgpu::TextureFormat::Rgba8Unorm,
        ResolvedColorSpace::Linear => wgpu::TextureFormat::Rgba8UnormSrgb,
    };
    let texture = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some(&format!("placeholder cubemap texture #{index}")),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 6,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: texture_format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        },
        TextureDataOrder::LayerMajor,
        &data,
    );

    let view = texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some(&format!("placeholder cubemap view #{index}")),
        dimension: Some(wgpu::TextureViewDimension::Cube),
        array_layer_count: Some(6),
        ..Default::default()
    });
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
        texture,
        view,
        sampler,
        resolution: [1.0, 1.0, 6.0, 0.0],
        keyboard: None,
    })
}

fn create_keyboard_channel(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    index: u32,
    color_space: ResolvedColorSpace,
) -> Result<ChannelResources> {
    let texture_format = match color_space {
        ResolvedColorSpace::Gamma => wgpu::TextureFormat::Rgba8Unorm,
        ResolvedColorSpace::Linear => wgpu::TextureFormat::Rgba8UnormSrgb,
    };

    let data = vec![
        0u8;
        (KEYBOARD_TEXTURE_WIDTH * KEYBOARD_TEXTURE_HEIGHT * KEYBOARD_BYTES_PER_PIXEL)
            as usize
    ];

    let texture = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some(&format!("keyboard channel texture #{index}")),
            size: wgpu::Extent3d {
                width: KEYBOARD_TEXTURE_WIDTH,
                height: KEYBOARD_TEXTURE_HEIGHT,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: texture_format,
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
        mag_filter: wgpu::FilterMode::Nearest,
        min_filter: wgpu::FilterMode::Nearest,
        mipmap_filter: wgpu::FilterMode::Nearest,
        ..Default::default()
    });

    Ok(ChannelResources {
        texture,
        view,
        sampler,
        resolution: [
            KEYBOARD_TEXTURE_WIDTH as f32,
            KEYBOARD_TEXTURE_HEIGHT as f32,
            1.0,
            0.0,
        ],
        keyboard: Some(KeyboardTextureMeta {
            width: KEYBOARD_TEXTURE_WIDTH,
            height: KEYBOARD_TEXTURE_HEIGHT,
        }),
    })
}

fn load_cubemap_channel(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    index: usize,
    directory: &Path,
    color_space: ResolvedColorSpace,
) -> Result<ChannelResources> {
    if !directory.exists() {
        anyhow::bail!(
            "cubemap directory {} does not exist for channel {}",
            directory.display(),
            index
        );
    }
    if !directory.is_dir() {
        anyhow::bail!(
            "cubemap path {} is not a directory for channel {}",
            directory.display(),
            index
        );
    }

    let mut faces = Vec::with_capacity(CUBEMAP_FACE_STEMS.len());
    for face in CUBEMAP_FACE_STEMS {
        let face_path = find_cubemap_face(directory, face).ok_or_else(|| {
            anyhow!(
                "cubemap face '{face}' missing for channel {index} in {}",
                directory.display()
            )
        })?;
        let image = image::open(&face_path).with_context(|| {
            format!(
                "failed to open cubemap face '{}' for channel {} at {}",
                face,
                index,
                face_path.display()
            )
        })?;
        let mut rgba = image.to_rgba8();
        flip_vertical_in_place(&mut rgba);
        faces.push((face_path, rgba));
    }

    let first = &faces[0].1;
    let width = first.width();
    let height = first.height();
    if width == 0 || height == 0 {
        anyhow::bail!(
            "cubemap face at {} has zero extent ({}x{})",
            faces[0].0.display(),
            width,
            height
        );
    }
    if width != height {
        anyhow::bail!(
            "cubemap faces must be square; {} is {}x{}",
            faces[0].0.display(),
            width,
            height
        );
    }

    for (path, image) in &faces[1..] {
        if image.width() != width || image.height() != height {
            anyhow::bail!(
                "cubemap face {} has mismatched resolution ({}x{} vs {}x{})",
                path.display(),
                image.width(),
                image.height(),
                width,
                height
            );
        }
    }

    let face_pixels = (width as usize) * (height as usize) * 4;
    let mut combined = Vec::with_capacity(face_pixels * faces.len());
    for (path, image) in faces {
        tracing::debug!(
            channel = index,
            face = %path.display(),
            width = image.width(),
            height = image.height(),
            "uploading cubemap face"
        );
        combined.extend_from_slice(image.as_raw());
    }

    let texture_format = match color_space {
        ResolvedColorSpace::Gamma => wgpu::TextureFormat::Rgba8Unorm,
        ResolvedColorSpace::Linear => wgpu::TextureFormat::Rgba8UnormSrgb,
    };

    let texture = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some(&format!("cubemap channel texture #{index}")),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 6,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: texture_format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        },
        TextureDataOrder::LayerMajor,
        &combined,
    );

    let view = texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some(&format!("cubemap channel view #{index}")),
        dimension: Some(wgpu::TextureViewDimension::Cube),
        array_layer_count: Some(6),
        ..Default::default()
    });
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });

    tracing::debug!(
        channel = index,
        dir = %directory.display(),
        width,
        height,
        "loaded cubemap channel"
    );

    Ok(ChannelResources {
        texture,
        view,
        sampler,
        resolution: [width as f32, height as f32, 6.0, 0.0],
        keyboard: None,
    })
}

fn load_texture_channel(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    index: usize,
    path: &Path,
    color_space: ResolvedColorSpace,
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

    let texture_format = match color_space {
        ResolvedColorSpace::Gamma => wgpu::TextureFormat::Rgba8Unorm,
        ResolvedColorSpace::Linear => wgpu::TextureFormat::Rgba8UnormSrgb,
    };

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
            format: texture_format,
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

    tracing::debug!(
        channel = index,
        path = %path.display(),
        width,
        height,
        "loaded texture channel"
    );

    Ok(ChannelResources {
        texture,
        view,
        sampler,
        resolution: [width as f32, height as f32, 1.0, 0.0],
        keyboard: None,
    })
}

fn find_cubemap_face(directory: &Path, face: &str) -> Option<PathBuf> {
    let target = face.to_ascii_lowercase();
    let entries = std::fs::read_dir(directory).ok()?;
    let mut candidates = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let stem = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().to_ascii_lowercase());
        if matches!(stem.as_deref(), Some(stem) if stem == target) {
            candidates.push(path);
        }
    }
    candidates.sort();
    candidates.into_iter().next()
}

struct ResolvedFrame {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

struct FrameCapture {
    buffer: wgpu::Buffer,
    width: u32,
    height: u32,
    bytes_per_row: u32,
    pixel_format: CapturePixelFormat,
}

enum CapturePixelFormat {
    Rgba,
    Bgra,
}

impl FrameCapture {
    fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        size: PhysicalSize<u32>,
    ) -> Result<Self> {
        let width = size.width.max(1);
        let height = size.height.max(1);
        if width == 0 || height == 0 {
            anyhow::bail!("cannot capture frame with zero-sized surface");
        }

        let pixel_format = match format {
            wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Rgba8UnormSrgb => {
                CapturePixelFormat::Rgba
            }
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb => {
                CapturePixelFormat::Bgra
            }
            other => anyhow::bail!("capture unsupported for surface format {other:?}"),
        };

        let bytes_per_pixel = 4u32;
        let unaligned = width
            .checked_mul(bytes_per_pixel)
            .ok_or_else(|| anyhow!("capture dimension overflow"))?;
        let bytes_per_row = align_to(unaligned, wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer_size = bytes_per_row as u64 * height as u64;

        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("wax11-frame-capture"),
            size: buffer_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        Ok(Self {
            buffer,
            width,
            height,
            bytes_per_row,
            pixel_format,
        })
    }

    fn encode_copy(&self, texture: &wgpu::Texture, encoder: &mut wgpu::CommandEncoder) {
        let layout = wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(self.bytes_per_row),
            rows_per_image: Some(self.height),
        };
        let extent = wgpu::Extent3d {
            width: self.width,
            height: self.height,
            depth_or_array_layers: 1,
        };
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.buffer,
                layout,
            },
            extent,
        );
    }

    fn resolve(self, device: &wgpu::Device) -> Result<ResolvedFrame> {
        let slice = self.buffer.slice(..);
        let (tx, rx) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        device
            .poll(wgpu::PollType::Wait)
            .map_err(|err| anyhow!("device poll failed while awaiting frame capture: {err:?}"))?;

        rx.recv()
            .context("failed to receive frame capture map result")?
            .map_err(|err| anyhow!("failed to map frame capture buffer: {err:?}"))?;

        let mapped = slice.get_mapped_range();
        let stride = self.bytes_per_row as usize;
        let row_len = (self.width as usize) * 4;
        let mut rgba = vec![0u8; row_len * self.height as usize];

        for row in 0..self.height as usize {
            let src_offset = row * stride;
            let dst_offset = row * row_len;
            let src_row = &mapped[src_offset..src_offset + row_len];
            let dst_row = &mut rgba[dst_offset..dst_offset + row_len];
            match self.pixel_format {
                CapturePixelFormat::Rgba => dst_row.copy_from_slice(src_row),
                CapturePixelFormat::Bgra => {
                    for (dst, src) in dst_row.chunks_exact_mut(4).zip(src_row.chunks_exact(4)) {
                        dst[0] = src[2];
                        dst[1] = src[1];
                        dst[2] = src[0];
                        dst[3] = src[3];
                    }
                }
            }
        }

        drop(mapped);
        self.buffer.unmap();

        flip_rgba_vertical(self.width, self.height, &mut rgba);

        Ok(ResolvedFrame {
            width: self.width,
            height: self.height,
            rgba,
        })
    }
}

fn align_to(value: u32, alignment: u32) -> u32 {
    if alignment == 0 {
        return value;
    }
    let mask = alignment - 1;
    match value.checked_add(mask) {
        Some(adj) => adj & !mask,
        None => value,
    }
}

fn flip_rgba_vertical(width: u32, height: u32, data: &mut [u8]) {
    let row_len = width as usize * 4;
    if row_len == 0 {
        return;
    }
    let rows = height as usize;
    for row in 0..(rows / 2) {
        let top_start = row * row_len;
        let bottom_start = (rows - 1 - row) * row_len;
        let (head, tail) = data.split_at_mut(bottom_start);
        let top_slice = &mut head[top_start..top_start + row_len];
        let bottom_slice = &mut tail[..row_len];
        top_slice.swap_with_slice(bottom_slice);
    }
}

impl FileExportTarget {
    fn write(&self, frame: &ResolvedFrame) -> Result<()> {
        match self.format {
            ExportFormat::Png => self.write_png(frame),
            ExportFormat::Exr => {
                anyhow::bail!("EXR export is not implemented yet; use a PNG path")
            }
        }
    }

    fn write_png(&self, frame: &ResolvedFrame) -> Result<()> {
        tracing::info!(
            path = %self.path.display(),
            width = frame.width,
            height = frame.height,
            bytes = frame.rgba.len(),
            "writing PNG export"
        );
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create directories for still export at {}",
                    self.path.display()
                )
            })?;
        }

        let file = File::create(&self.path)
            .with_context(|| format!("failed to create export file at {}", self.path.display()))?;
        let mut writer = BufWriter::new(file);
        {
            let encoder = PngEncoder::new(&mut writer);
            encoder
                .write_image(
                    &frame.rgba,
                    frame.width,
                    frame.height,
                    ExtendedColorType::Rgba8,
                )
                .context("failed to encode PNG")?;
        }
        writer.flush().context("failed to flush PNG writer")?;
        let final_size = std::fs::metadata(&self.path)
            .map(|meta| meta.len())
            .unwrap_or(0);
        tracing::info!(size = final_size, path = %self.path.display(), "finished writing export");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{align_of, size_of};

    #[test]
    fn shadertoy_uniforms_follow_std140_layout() {
        let uniforms = ShadertoyUniforms::new(1920, 1080);
        let base = &uniforms as *const _ as usize;

        assert_eq!(align_of::<ShadertoyUniforms>(), 16);
        assert_eq!(size_of::<ShadertoyUniforms>(), 256);
        assert_eq!((&uniforms.i_resolution as *const _ as usize) - base, 0);
        assert_eq!((&uniforms.i_time as *const _ as usize) - base, 16);
        assert_eq!((&uniforms.i_time_delta as *const _ as usize) - base, 20);
        assert_eq!((&uniforms.i_frame as *const _ as usize) - base, 24);
        assert_eq!((&uniforms.i_mouse as *const _ as usize) - base, 32);
        assert_eq!((&uniforms.i_date as *const _ as usize) - base, 48);
        assert_eq!((&uniforms.i_sample_rate as *const _ as usize) - base, 64);
        assert_eq!((&uniforms.i_fade as *const _ as usize) - base, 68);
        assert_eq!((&uniforms.i_channel_time as *const _ as usize) - base, 80);
        assert_eq!(
            (&uniforms.i_channel_resolution as *const _ as usize) - base,
            144
        );
        assert_eq!((&uniforms.i_surface as *const _ as usize) - base, 208);
        assert_eq!((&uniforms.i_fill as *const _ as usize) - base, 224);
        assert_eq!((&uniforms.i_fill_wrap as *const _ as usize) - base, 240);
    }

    #[test]
    fn crossfade_weights_sum_to_one() {
        let mut fade = CrossfadeState::new(2);
        // initial state: 100% previous, 0% current
        let (prev_start, curr_start) = fade.mixes();
        assert!((prev_start - 1.0).abs() < 1e-5);
        assert!(curr_start.abs() < 1e-5);

        fade.advance();
        let (prev_mid, curr_mid) = fade.mixes();
        assert!((prev_mid + curr_mid - 1.0).abs() < 1e-5);
        assert!((prev_mid - 0.5).abs() < 1e-5);
        assert!((curr_mid - 0.5).abs() < 1e-5);

        fade.advance();
        let (prev_end, curr_end) = fade.mixes();
        assert!(fade.is_finished());
        assert!(prev_end.abs() < 1e-5);
        assert!((curr_end - 1.0).abs() < 1e-5);
    }

    #[test]
    fn zero_duration_crossfade_is_hard_cut() {
        let mut fade = CrossfadeState::new(1);
        let (prev_start, curr_start) = fade.mixes();
        assert!((prev_start - 1.0).abs() < 1e-5);
        assert!(curr_start.abs() < 1e-5);

        fade.advance();
        let (prev_end, curr_end) = fade.mixes();
        assert!(fade.is_finished());
        assert!(prev_end.abs() < 1e-5);
        assert!((curr_end - 1.0).abs() < 1e-5);
    }
}
