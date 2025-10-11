use std::collections::HashMap;
use std::fmt;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use tracing::{debug, warn};
use wgpu::util::DeviceExt;
use winit::dpi::PhysicalSize;

use crate::runtime::{FillMethod, TimeSample};
use crate::types::{
    AdapterProfile, Antialiasing, ChannelBindings, ChannelTextureKind, ColorSpaceMode,
    CrossfadeCurve, GpuMemoryMode, GpuPowerPreference, ShaderCompiler, VsyncMode, CHANNEL_COUNT,
};

use super::channels::{KEYBOARD_BYTES_PER_PIXEL, KEYBOARD_TEXTURE_HEIGHT, KEYBOARD_TEXTURE_WIDTH};
use super::context::{GpuContext, SurfaceColorSpace};
use super::pipeline::{PipelineLayouts, ShaderPipeline};
use super::timeline::FadeEnvelope;
use super::uniforms::{fill_parameters, logical_dimensions, ShadertoyUniforms};

const PIPELINE_CACHE_MODE: PipelineCacheMode = PipelineCacheMode::OnDemand;
const PIPELINE_BUILD_STRATEGY: PipelineBuildStrategy = PipelineBuildStrategy::Threaded;
const PIPELINE_PRIME_ON_SUBMIT: bool = true;

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PipelineCacheMode {
    Disabled,
    OnDemand,
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PipelineBuildStrategy {
    Immediate,
    Threaded,
}

#[derive(Clone)]
struct PipelineHandle(Arc<ShaderPipeline>);

impl PipelineHandle {
    fn from_pipeline(pipeline: ShaderPipeline) -> Self {
        Self(Arc::new(pipeline))
    }

    fn shader_path(&self) -> &Path {
        &self.0.shader_source
    }
}

impl Deref for PipelineHandle {
    type Target = ShaderPipeline;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct PipelineKey {
    shader_path: PathBuf,
    channel_signature: u8,
    compiler: ShaderCompiler,
    sample_count: u32,
    surface_format: wgpu::TextureFormat,
    linear_color_space: bool,
}

impl PipelineKey {
    fn new(
        shader_path: &Path,
        channel_signature: u8,
        compiler: ShaderCompiler,
        sample_count: u32,
        surface_format: wgpu::TextureFormat,
        color_space: SurfaceColorSpace,
    ) -> Self {
        Self {
            shader_path: shader_path.to_path_buf(),
            channel_signature,
            compiler,
            sample_count,
            surface_format,
            linear_color_space: matches!(color_space, SurfaceColorSpace::Linear),
        }
    }
}

struct PipelineCache {
    mode: PipelineCacheMode,
    entries: HashMap<PipelineKey, PipelineHandle>,
}

impl PipelineCache {
    fn new(mode: PipelineCacheMode) -> Self {
        Self {
            mode,
            entries: HashMap::new(),
        }
    }

    fn enabled(&self) -> bool {
        !matches!(self.mode, PipelineCacheMode::Disabled)
    }

    fn get(&self, key: &PipelineKey) -> Option<PipelineHandle> {
        self.entries.get(key).cloned()
    }

    fn store(&mut self, key: PipelineKey, handle: PipelineHandle) {
        if self.enabled() {
            self.entries.insert(key, handle);
        }
    }
}

enum PipelineFuture {
    Ready(PipelineHandle),
    Threaded {
        receiver: Receiver<anyhow::Result<PipelineHandle>>,
    },
}

impl PipelineFuture {
    fn ready(handle: PipelineHandle) -> Self {
        PipelineFuture::Ready(handle)
    }

    fn poll(&mut self) -> anyhow::Result<Option<PipelineHandle>> {
        match self {
            PipelineFuture::Ready(handle) => Ok(Some(handle.clone())),
            PipelineFuture::Threaded { receiver } => match receiver.try_recv() {
                Ok(result) => result.map(Some),
                Err(TryRecvError::Empty) => Ok(None),
                Err(TryRecvError::Disconnected) => Err(anyhow!(
                    "pipeline build worker disconnected before returning a result"
                )),
            },
        }
    }
}

fn compute_channel_signature(kinds: &[ChannelTextureKind; CHANNEL_COUNT]) -> u8 {
    kinds.iter().enumerate().fold(0u8, |acc, (index, kind)| {
        let bit = matches!(kind, ChannelTextureKind::Cubemap) as u8;
        acc | (bit << index)
    })
}

#[derive(Debug, Clone)]
pub struct FileExportTarget {
    pub path: PathBuf,
    pub _format: crate::runtime::ExportFormat,
}

#[derive(Debug)]
pub enum RenderExportError {
    Surface(wgpu::SurfaceError),
    Unsupported,
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
            RenderExportError::Unsupported => write!(f, "still-frame export is not implemented"),
        }
    }
}

impl std::error::Error for RenderExportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            RenderExportError::Surface(err) => Some(err),
            RenderExportError::Unsupported => None,
        }
    }
}

pub(crate) struct GpuState {
    context: GpuContext,
    layouts: PipelineLayouts,
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    uniforms: ShadertoyUniforms,
    channel_kinds: [ChannelTextureKind; CHANNEL_COUNT],
    shader_compiler: ShaderCompiler,
    render_scale: f32,
    fill_method: FillMethod,
    crossfade_curve: CrossfadeCurve,
    current: PipelineHandle,
    previous: Option<PipelineHandle>,
    pending: Option<PendingPipeline>,
    pipeline_cache: PipelineCache,
    fade: Option<FadeEnvelope>,
    multisample_target: Option<MultisampleTarget>,
    start_time: Instant,
    last_frame_time: Instant,
    frame_count: u32,
    last_override_sample: Option<TimeSample>,
    last_fps_update: Instant,
    frames_since_last_update: u32,
    frames_per_second: f32,
    vsync_mode: VsyncMode,
    is_crossfading: bool,
}

struct PendingPipeline {
    key: PipelineKey,
    future: PipelineFuture,
    handle: Option<PipelineHandle>,
    crossfade: Duration,
    warmup: Duration,
    warmup_deadline: Option<Instant>,
    warmed: bool,
}

impl PendingPipeline {
    fn new(
        key: PipelineKey,
        future: PipelineFuture,
        crossfade: Duration,
        warmup: Duration,
    ) -> Self {
        Self {
            key,
            future,
            handle: None,
            crossfade,
            warmup,
            warmup_deadline: None,
            warmed: false,
        }
    }

    fn from_ready(
        key: PipelineKey,
        handle: PipelineHandle,
        crossfade: Duration,
        warmup: Duration,
        now: Instant,
    ) -> Self {
        let mut pending = Self::new(
            key,
            PipelineFuture::ready(handle.clone()),
            crossfade,
            warmup,
        );
        pending.mark_ready(handle, now);
        pending
    }

    fn poll(&mut self, cache: &mut PipelineCache, now: Instant) -> anyhow::Result<bool> {
        if self.handle.is_some() {
            return Ok(true);
        }

        if let Some(handle) = self.future.poll()? {
            cache.store(self.key.clone(), handle.clone());
            self.mark_ready(handle, now);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn mark_ready(&mut self, handle: PipelineHandle, now: Instant) {
        self.handle = Some(handle);
        if self.warmup_deadline.is_none() {
            self.warmup_deadline = Some(now + self.warmup);
        }
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
            label: Some("msaa color target"),
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
        crossfade_curve: CrossfadeCurve,
        vsync_mode: VsyncMode,
    ) -> Result<Self>
    where
        T: HasDisplayHandle + HasWindowHandle,
    {
        let context = GpuContext::new(
            target,
            initial_size,
            antialiasing,
            color_space,
            gpu_power,
            gpu_memory,
            gpu_latency,
            vsync_mode,
        )?;
        let channel_kinds = channel_bindings.layout_signature();
        let layouts = PipelineLayouts::new(&context.device, shader_compiler)?;

        let uniform_buffer = context.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("uniform buffer"),
            size: std::mem::size_of::<ShadertoyUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform_bind_group = context
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("uniform bind group"),
                layout: &layouts.uniform_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                }],
            });

        let current = ShaderPipeline::new(
            &context.device,
            &context.queue,
            &layouts,
            context.surface_format,
            context.sample_count,
            shader_source,
            channel_bindings,
            &channel_kinds,
            context.color_space,
            shader_compiler,
        )?;

        let current = PipelineHandle::from_pipeline(current);
        let mut pipeline_cache = PipelineCache::new(PIPELINE_CACHE_MODE);
        if pipeline_cache.enabled() {
            let key = PipelineKey::new(
                shader_source,
                compute_channel_signature(&channel_kinds),
                shader_compiler,
                context.sample_count,
                context.surface_format,
                context.color_space,
            );
            pipeline_cache.store(key, current.clone());
        }

        let mut uniforms = ShadertoyUniforms::new(context.size.width, context.size.height);
        uniforms.set_fade(1.0);
        Self::write_uniforms(&context.queue, &uniform_buffer, &uniforms);

        let multisample_target = if context.sample_count > 1 {
            Some(MultisampleTarget::new(
                &context.device,
                context.surface_format,
                context.size,
                context.sample_count,
            ))
        } else {
            None
        };

        Ok(Self {
            context,
            layouts,
            uniform_buffer,
            uniform_bind_group,
            uniforms,
            channel_kinds,
            shader_compiler,
            render_scale,
            fill_method,
            crossfade_curve,
            current,
            previous: None,
            pending: None,
            pipeline_cache,
            fade: None,
            multisample_target,
            start_time: Instant::now(),
            last_frame_time: Instant::now(),
            frame_count: 0,
            last_override_sample: None,
            last_fps_update: Instant::now(),
            frames_since_last_update: 0,
            frames_per_second: 60.0,
            vsync_mode,
            is_crossfading: false,
        })
    }

    pub(crate) fn size(&self) -> PhysicalSize<u32> {
        self.context.size
    }

    pub(crate) fn channel_kinds(&self) -> &[ChannelTextureKind; CHANNEL_COUNT] {
        &self.channel_kinds
    }

    pub(crate) fn adapter_profile(&self) -> &AdapterProfile {
        &self.context.adapter_profile
    }

    pub(crate) fn has_keyboard_channel(&self) -> bool {
        self.current.has_keyboard_channel()
            || self
                .previous
                .as_ref()
                .map(|pipeline| pipeline.has_keyboard_channel())
                .unwrap_or(false)
            || self
                .pending
                .as_ref()
                .and_then(|pending| pending.handle.as_ref())
                .map(|handle| handle.has_keyboard_channel())
                .unwrap_or(false)
    }

    pub(crate) fn update_keyboard_channels(&self, data: &[u8]) {
        // Ignore mis-sized payloads early; this keeps the number of warn logs down.
        let expected_len =
            (KEYBOARD_TEXTURE_WIDTH * KEYBOARD_TEXTURE_HEIGHT * KEYBOARD_BYTES_PER_PIXEL) as usize;
        if data.len() != expected_len {
            return;
        }
        let queue = &self.context.queue;
        self.current.update_keyboard_channels(queue, data);
        if let Some(previous) = &self.previous {
            previous.update_keyboard_channels(queue, data);
        }
        if let Some(pending) = &self.pending {
            if let Some(handle) = &pending.handle {
                handle.update_keyboard_channels(queue, data);
            }
        }
    }

    pub(crate) fn resize(&mut self, new_size: PhysicalSize<u32>) {
        if new_size.width == 0 || new_size.height == 0 {
            return;
        }
        self.context.resize(new_size);
        self.uniforms
            .set_resolution(new_size.width as f32, new_size.height as f32);
        self.uniforms.set_surface(
            new_size.width as f32,
            new_size.height as f32,
            new_size.width as f32,
            new_size.height as f32,
        );
        self.multisample_target = if self.context.sample_count > 1 {
            Some(MultisampleTarget::new(
                &self.context.device,
                self.context.surface_format,
                self.context.size,
                self.context.sample_count,
            ))
        } else {
            None
        };
    }

    pub(crate) fn set_shader(
        &mut self,
        shader_source: &Path,
        channel_bindings: &ChannelBindings,
        crossfade: Duration,
        warmup: Duration,
        now: Instant,
        curve: CrossfadeCurve,
    ) -> Result<()> {
        let layout_signature = channel_bindings.layout_signature();
        if layout_signature != self.channel_kinds {
            anyhow::bail!("channel layout changed; rebuild renderer to account for new resources");
        }

        let key = PipelineKey::new(
            shader_source,
            compute_channel_signature(&self.channel_kinds),
            self.shader_compiler,
            self.context.sample_count,
            self.context.surface_format,
            self.context.color_space,
        );

        self.crossfade_curve = curve;

        if let Some(handle) = self.pipeline_cache.get(&key) {
            self.pending = Some(PendingPipeline::from_ready(
                key, handle, crossfade, warmup, now,
            ));
            return Ok(());
        }

        match PIPELINE_BUILD_STRATEGY {
            PipelineBuildStrategy::Immediate => {
                let handle = self.build_pipeline_immediate(shader_source, channel_bindings)?;
                self.pipeline_cache.store(key.clone(), handle.clone());
                self.pending = Some(PendingPipeline::from_ready(
                    key, handle, crossfade, warmup, now,
                ));
            }
            PipelineBuildStrategy::Threaded => {
                let future = self
                    .build_pipeline_threaded(shader_source.to_path_buf(), channel_bindings.clone());
                self.pending = Some(PendingPipeline::new(key, future, crossfade, warmup));
            }
        }
        Ok(())
    }

    fn build_pipeline_immediate(
        &self,
        shader_source: &Path,
        channel_bindings: &ChannelBindings,
    ) -> Result<PipelineHandle> {
        let pipeline = ShaderPipeline::new(
            &self.context.device,
            &self.context.queue,
            &self.layouts,
            self.context.surface_format,
            self.context.sample_count,
            shader_source,
            channel_bindings,
            &self.channel_kinds,
            self.context.color_space,
            self.shader_compiler,
        )?;
        Ok(PipelineHandle::from_pipeline(pipeline))
    }

    fn build_pipeline_threaded(
        &self,
        shader_path: PathBuf,
        channel_bindings: ChannelBindings,
    ) -> PipelineFuture {
        let device = self.context.device.clone();
        let queue = self.context.queue.clone();
        let layouts = self.layouts.clone();
        let surface_format = self.context.surface_format;
        let sample_count = self.context.sample_count;
        let channel_kinds = self.channel_kinds;
        let color_space = self.context.color_space;
        let compiler = self.shader_compiler;

        let (sender, receiver) = mpsc::channel();

        thread::spawn(move || {
            let result = ShaderPipeline::new(
                &device,
                &queue,
                &layouts,
                surface_format,
                sample_count,
                shader_path.as_path(),
                &channel_bindings,
                &channel_kinds,
                color_space,
                compiler,
            )
            .map(PipelineHandle::from_pipeline);
            let _ = sender.send(result);
        });

        PipelineFuture::Threaded { receiver }
    }

    pub(crate) fn render(
        &mut self,
        mouse: [f32; 4],
        time_sample: Option<TimeSample>,
    ) -> Result<(), wgpu::SurfaceError> {
        let frame = self.render_internal(mouse, time_sample)?;
        frame.present();
        Ok(())
    }

    pub(crate) fn render_export(
        &mut self,
        _mouse: [f32; 4],
        _time_sample: Option<TimeSample>,
        _target: &FileExportTarget,
    ) -> Result<PathBuf, RenderExportError> {
        Err(RenderExportError::Unsupported)
    }

    fn render_internal(
        &mut self,
        mouse: [f32; 4],
        time_sample: Option<TimeSample>,
    ) -> Result<wgpu::SurfaceTexture, wgpu::SurfaceError> {
        // Acquire the next frame texture early. This call can block, so we do it before
        // handling shader transitions to avoid compounding delays.
        let frame_acquisition_start = Instant::now();
        let frame = self.context.surface.get_current_texture()?;
        let frame_acquisition_duration = frame_acquisition_start.elapsed();
        let frame_time_budget = Duration::from_secs_f32(1.0 / self.frames_per_second);

        if frame_acquisition_duration > frame_time_budget {
            warn!(
                "acquiring frame took {}ms, which is over the frame budget of {}ms (at {} FPS)",
                frame_acquisition_duration.as_millis(),
                frame_time_budget.as_millis(),
                self.frames_per_second.round(),
            );
        }

        let now = Instant::now();
        self.frames_since_last_update += 1;
        let elapsed_since_fps_update = now.saturating_duration_since(self.last_fps_update);
        if elapsed_since_fps_update >= Duration::from_secs(1) {
            self.frames_per_second =
                self.frames_since_last_update as f32 / elapsed_since_fps_update.as_secs_f32();
            self.frames_since_last_update = 0;
            self.last_fps_update = now;
            debug!(
                fps = self.frames_per_second.round(),
                frame_count = self.frame_count,
                time = self.uniforms.i_time,
                crossfading = self.fade.is_some(),
                pending = self.pending.is_some(),
                "render stats"
            );
        }

        self.uniforms.update_time(
            &mut self.start_time,
            &mut self.last_frame_time,
            &mut self.frame_count,
            &mut self.last_override_sample,
            now,
            time_sample,
            mouse,
        );

        let mut ready_pending = None;
        if let Some(mut pending) = self.pending.take() {
            match pending.poll(&mut self.pipeline_cache, now) {
                Ok(true) => ready_pending = Some(pending),
                Ok(false) => self.pending = Some(pending),
                Err(err) => {
                    warn!(error = %err, "failed to resolve pending pipeline");
                }
            }
        }

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder =
            self.context
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("render encoder"),
                });

        let current_pipeline = self.current.clone();

        let mut load = wgpu::LoadOp::Clear(wgpu::Color::BLACK);
        let mut previous_pipeline = self.previous.take();
        let mut fade_state = self.fade.take();

        // Track whether we're starting or ending a crossfade
        let was_crossfading = self.is_crossfading;

        if let (Some(previous), Some(fade)) = (previous_pipeline.as_ref(), fade_state.as_mut()) {
            let (prev_mix, curr_mix, finished) = fade.mixes(now);

            // If this is the start of crossfade and mode is Crossfade, disable vsync
            if !was_crossfading && matches!(self.vsync_mode, VsyncMode::Crossfade) {
                self.context.set_vsync(false);
            }

            if prev_mix > f32::EPSILON {
                self.encode_draw(&mut encoder, &view, previous, prev_mix, load);
                load = wgpu::LoadOp::Load;
            }
            if curr_mix > f32::EPSILON {
                self.encode_draw(&mut encoder, &view, &current_pipeline, curr_mix, load);
            }
            if finished {
                previous_pipeline = None;
                fade_state = None;

                // Re-enable vsync when crossfade finishes
                if matches!(self.vsync_mode, VsyncMode::Crossfade) {
                    self.context.set_vsync(true);
                }
            }
        } else {
            self.encode_draw(&mut encoder, &view, &current_pipeline, 1.0, load);
            previous_pipeline = None;
            fade_state = None;
        }

        self.is_crossfading = fade_state.is_some();
        self.previous = previous_pipeline;
        self.fade = fade_state;

        if let Some(mut pending) = ready_pending {
            if let Some(handle) = pending.handle.as_ref() {
                if PIPELINE_PRIME_ON_SUBMIT && !pending.warmed {
                    // Pre-warm the pending pipeline by encoding a draw with zero mix. This forces the
                    // driver to compile the shader and allocate resources before the crossfade begins,
                    // preventing a stutter on the first frame of the transition.
                    let prewarm_start = Instant::now();
                    self.encode_draw(&mut encoder, &view, handle, 0.0, wgpu::LoadOp::Load);
                    let prewarm_duration = prewarm_start.elapsed();
                    debug!(
                        shader = %handle.shader_path().display(),
                        duration_us = prewarm_duration.as_micros(),
                        frame_acquisition_duration_us = frame_acquisition_duration.as_micros(),
                        "pre-warmed new shader pipeline"
                    );
                    pending.warmed = true;
                }

                if pending
                    .warmup_deadline
                    .is_none_or(|deadline| now >= deadline)
                {
                    self.promote_pending(pending, now);
                } else {
                    self.pending = Some(pending);
                }
            } else {
                // Handle should always be ready in this branch, but keep the pipeline pending if not.
                self.pending = Some(pending);
            }
        }

        self.context.queue.submit(std::iter::once(encoder.finish()));

        Ok(frame)
    }

    fn draw_pipeline(&mut self, pipeline: &ShaderPipeline, mix: f32) {
        for (index, resource) in pipeline.channel_resources.iter().enumerate() {
            self.uniforms
                .set_channel_resolution(index, resource.resolution);
        }
        self.uniforms.set_fade(mix);
        let logical = logical_dimensions(self.render_scale, self.fill_method, self.context.size);
        let (scale_x, scale_y, offset_x, offset_y, wrap_x, wrap_y) = fill_parameters(
            self.render_scale,
            self.fill_method,
            self.context.size,
            logical,
        );
        self.uniforms.set_resolution(logical.0, logical.1);
        self.uniforms.set_surface(
            self.context.size.width as f32,
            self.context.size.height as f32,
            logical.0,
            logical.1,
        );
        self.uniforms.set_fill(scale_x, scale_y, offset_x, offset_y);
        self.uniforms.set_fill_wrap(wrap_x, wrap_y);
        // Note: per-pass uniform upload is performed inside encode_draw via a staging copy
    }

    fn encode_draw(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        pipeline: &ShaderPipeline,
        mix: f32,
        load: wgpu::LoadOp<wgpu::Color>,
    ) {
        self.draw_pipeline(pipeline, mix);

        // Pre-warming: a `mix` of 0.0 indicates that this pass is only for compiling the
        // shader and allocating resources. We submit the uniform buffer write and bind
        // the pipeline, but skip the draw call itself. This avoids a stutter on the first
        // frame of a crossfade.
        let is_prewarming = mix <= 0.0;

        // Upload uniforms for this pass via a staging buffer and copy on the encoder so
        // each pass sees its own uniform values (prevents crossfade mix bleeding).
        let staging = self
            .context
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
        let (attachment_view, resolve_target) = if let Some(msaa) = self.multisample_target.as_ref()
        {
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

        if !is_prewarming {
            render_pass.draw(0..3, 0..1);
        }
    }

    fn promote_pending(&mut self, pending: PendingPipeline, now: Instant) {
        let handle = pending
            .handle
            .expect("pending pipeline promoted without a resolved handle");

        if pending.crossfade <= Duration::from_millis(16) {
            self.current = handle;
            self.previous = None;
            self.fade = None;
            return;
        }

        let previous = std::mem::replace(&mut self.current, handle);
        self.previous = Some(previous);
        self.fade = FadeEnvelope::new(pending.crossfade, self.crossfade_curve, now);
    }

    fn write_uniforms(queue: &wgpu::Queue, buffer: &wgpu::Buffer, uniforms: &ShadertoyUniforms) {
        queue.write_buffer(buffer, 0, bytemuck::bytes_of(uniforms));
    }
}
