use std::collections::HashMap;
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::result::Result as StdResult;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use crossbeam_channel::{self, Receiver, Sender};
use smithay_client_toolkit::reexports::client::{
    globals::registry_queue_init,
    protocol::{wl_output, wl_surface},
    Connection, Proxy, QueueHandle,
};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, Region},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry,
    output::{OutputHandler, OutputInfo, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::wlr_layer::{
        Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
        LayerSurfaceConfigure,
    },
};
use wgpu::SurfaceError;
use winit::dpi::PhysicalSize;

use crate::gpu::GpuState;
use crate::types::{Antialiasing, ChannelBindings, RendererConfig, ShaderCompiler, SurfaceAlpha};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SurfaceId(u64);

impl SurfaceId {
    pub fn raw(self) -> u64 {
        self.0
    }
}

impl From<u64> for SurfaceId {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl From<SurfaceId> for u64 {
    fn from(value: SurfaceId) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OutputId(u64);

impl OutputId {
    pub fn raw(self) -> u64 {
        self.0
    }
}

impl From<u64> for OutputId {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl From<OutputId> for u64 {
    fn from(value: OutputId) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceSelector {
    All,
    Surface(SurfaceId),
    Output(OutputId),
}

#[derive(Debug, Clone)]
pub struct SurfaceInfo {
    pub surface_id: SurfaceId,
    pub output_id: Option<OutputId>,
    pub output_name: Option<String>,
    pub size: Option<(u32, u32)>,
}

pub struct WallpaperRuntime {
    sender: Sender<WallpaperCommand>,
    join_handle: Option<JoinHandle<Result<()>>>,
}

#[derive(Debug, Clone)]
pub struct SwapRequest {
    pub shader_source: PathBuf,
    pub channel_bindings: ChannelBindings,
    pub crossfade: Duration,
    pub target_fps: Option<f32>,
    pub antialiasing: Antialiasing,
    pub surface_alpha: SurfaceAlpha,
    pub warmup: Duration,
}

impl WallpaperRuntime {
    pub fn spawn(config: RendererConfig) -> Result<Self> {
        let (sender, receiver) = crossbeam_channel::unbounded();
        let handle = thread::Builder::new()
            .name("hyshadew-wallpaper".into())
            .spawn(move || run_internal(config, receiver))
            .context("failed to spawn wallpaper thread")?;

        Ok(Self {
            sender,
            join_handle: Some(handle),
        })
    }

    pub fn surfaces(&self) -> Result<Vec<SurfaceInfo>> {
        let (tx, rx) = crossbeam_channel::bounded(1);
        self.sender
            .send(WallpaperCommand::QuerySurfaces { responder: tx })
            .map_err(|err| anyhow!("failed to send surface query: {err}"))?;
        rx.recv()
            .map_err(|err| anyhow!("failed to receive surface query response: {err}"))
    }

    pub fn swap_shader(&self, selector: SurfaceSelector, request: SwapRequest) -> Result<()> {
        self.sender
            .send(WallpaperCommand::SwapShader { selector, request })
            .map_err(|err| anyhow!("failed to send swap command: {err}"))
    }

    pub fn swap_shader_all(&self, request: SwapRequest) -> Result<()> {
        self.swap_shader(SurfaceSelector::All, request)
    }

    pub fn shutdown(mut self) -> Result<()> {
        if let Some(handle) = self.join_handle.take() {
            // Best-effort shutdown; ignore errors if the runtime already exited.
            let _ = self.sender.send(WallpaperCommand::Shutdown);
            let join_result = handle
                .join()
                .map_err(|err| anyhow!("wallpaper thread panicked: {err:?}"))?;
            join_result?;
        }
        Ok(())
    }
}

impl Drop for WallpaperRuntime {
    fn drop(&mut self) {
        if let Some(handle) = self.join_handle.take() {
            let _ = self.sender.send(WallpaperCommand::Shutdown);
            let _ = handle.join();
        }
    }
}

enum WallpaperCommand {
    SwapShader {
        selector: SurfaceSelector,
        request: SwapRequest,
    },
    QuerySurfaces {
        responder: Sender<Vec<SurfaceInfo>>,
    },
    Shutdown,
}

pub(crate) fn run(config: &RendererConfig) -> Result<()> {
    run_internal(config.clone(), crossbeam_channel::never())
}

fn run_internal(config: RendererConfig, command_rx: Receiver<WallpaperCommand>) -> Result<()> {
    let conn = Connection::connect_to_env().context("failed to connect to Wayland compositor")?;
    let (globals, mut event_queue) =
        registry_queue_init(&conn).context("failed to initialize Wayland registry queue")?;
    let qh = event_queue.handle();

    let compositor =
        CompositorState::bind(&globals, &qh).context("wl_compositor is not available")?;
    let layer_shell =
        LayerShell::bind(&globals, &qh).context("layer shell protocol is not available")?;

    let registry_state = RegistryState::new(&globals);
    let output_state = OutputState::new(&globals, &qh);

    let mut manager = WallpaperManager::new(
        compositor,
        layer_shell,
        registry_state,
        output_state,
        &config,
    );
    manager.initialise_surfaces(&conn, &qh)?;

    loop {
        // Drain any pending commands before touching the Wayland queue.
        while let Ok(command) = command_rx.try_recv() {
            manager.handle_command(command, &conn, &qh);
        }

        if manager.should_exit() {
            break;
        }

        event_queue
            .dispatch_pending(&mut manager)
            .context("error while processing pending Wayland events")?;

        while let Ok(command) = command_rx.try_recv() {
            manager.handle_command(command, &conn, &qh);
        }

        if manager.should_exit() {
            break;
        }

        match command_rx.recv_timeout(Duration::from_millis(5)) {
            Ok(command) => {
                manager.handle_command(command, &conn, &qh);
                continue;
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                manager.request_exit();
                break;
            }
        }

        if manager.should_exit() {
            break;
        }

        event_queue
            .blocking_dispatch(&mut manager)
            .context("error while processing Wayland events")?;

        if manager.should_exit() {
            break;
        }
    }

    Ok(())
}

struct WallpaperManager {
    compositor: CompositorState,
    layer_shell: LayerShell,
    registry_state: RegistryState,
    output_state: OutputState,
    surfaces: HashMap<SurfaceId, SurfaceState>,
    fallback_size: PhysicalSize<u32>,
    requested_size: Option<(u32, u32)>,
    shader_source: PathBuf,
    channel_bindings: ChannelBindings,
    antialiasing: Antialiasing,
    surface_alpha: SurfaceAlpha,
    target_fps: Option<f32>,
    shader_compiler: ShaderCompiler,
    should_exit: bool,
}

impl WallpaperManager {
    fn new(
        compositor: CompositorState,
        layer_shell: LayerShell,
        registry_state: RegistryState,
        output_state: OutputState,
        config: &RendererConfig,
    ) -> Self {
        Self {
            compositor,
            layer_shell,
            registry_state,
            output_state,
            surfaces: HashMap::new(),
            fallback_size: PhysicalSize::new(config.surface_size.0, config.surface_size.1),
            requested_size: config.requested_size,
            shader_source: config.shader_source.clone(),
            channel_bindings: config.channel_bindings.clone(),
            antialiasing: config.antialiasing,
            surface_alpha: config.surface_alpha,
            target_fps: config.target_fps,
            shader_compiler: config.shader_compiler,
            should_exit: false,
        }
    }

    fn initialise_surfaces(&mut self, conn: &Connection, qh: &QueueHandle<Self>) -> Result<()> {
        let mut created = false;
        for output in self.output_state.outputs() {
            self.ensure_surface_for_output(conn, qh, Some(output.clone()))?;
            created = true;
        }

        if !created {
            self.ensure_surface_for_output(conn, qh, None)?;
        }

        Ok(())
    }

    fn should_exit(&self) -> bool {
        self.should_exit
    }

    fn ensure_surface_for_output(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        output: Option<wl_output::WlOutput>,
    ) -> Result<()> {
        if let Some(ref out) = output {
            let key = proxy_key(out);
            if let Some(existing) = self
                .surfaces
                .values_mut()
                .find(|surface| surface.output_key == Some(key))
            {
                existing.last_output_size = output
                    .as_ref()
                    .and_then(|o| self.output_state.info(o))
                    .and_then(output_info_physical_size);
                if let Some(size) = existing.last_output_size {
                    let _ = existing.ensure_gpu(conn, &self.compositor, size);
                }
                return Ok(());
            }
        }

        let wl_surface = self.compositor.create_surface(qh);
        let layer_surface = self.layer_shell.create_layer_surface(
            qh,
            wl_surface,
            Layer::Background,
            Some("shaderpaper".to_string()),
            output.as_ref(),
        );
        layer_surface.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer_surface.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer_surface.set_exclusive_zone(-1);
        if let Some((width, height)) = self.requested_size {
            layer_surface.set_size(width, height);
        }
        layer_surface.commit();

        let initial_size = output
            .as_ref()
            .and_then(|out| self.output_state.info(out))
            .and_then(output_info_physical_size);
        let output_key = output.as_ref().map(proxy_key);
        let key = surface_key(layer_surface.wl_surface());
        let surface_state = SurfaceState::new(
            layer_surface,
            self.shader_source.clone(),
            self.channel_bindings.clone(),
            self.target_fps,
            output_key,
            initial_size,
            self.antialiasing,
            self.surface_alpha,
            self.shader_compiler,
        );
        self.surfaces.insert(key, surface_state);

        // Ensure GPU immediately if we already know the size.
        if let Some(size) = initial_size {
            if let Some(surface) = self.surfaces.get_mut(&key) {
                surface.ensure_gpu(conn, &self.compositor, size)?;
            }
        }

        Ok(())
    }

    fn resolve_configure_size(&self, output_size: Option<PhysicalSize<u32>>) -> PhysicalSize<u32> {
        let mut size = output_size.unwrap_or(self.fallback_size);
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
        size
    }

    fn request_exit(&mut self) {
        self.should_exit = true;
    }

    fn handle_command(
        &mut self,
        command: WallpaperCommand,
        conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match command {
            WallpaperCommand::SwapShader { selector, request } => {
                let SwapRequest {
                    shader_source,
                    channel_bindings,
                    crossfade,
                    target_fps,
                    antialiasing,
                    surface_alpha,
                    warmup,
                } = request;
                let now = Instant::now();
                for surface_id in self.target_surface_ids(&selector) {
                    if let Some(mut surface) = self.surfaces.remove(&surface_id) {
                        surface.apply_render_preferences(target_fps, antialiasing, surface_alpha);
                        let size = surface.last_output_size.unwrap_or(self.fallback_size);
                        if let Err(err) = surface.ensure_gpu(conn, &self.compositor, size) {
                            tracing::error!(
                                error = ?err,
                                "failed to prepare GPU before swap"
                            );
                        } else if let Err(err) = surface.set_shader(
                            now,
                            shader_source.as_path(),
                            &channel_bindings,
                            crossfade,
                            warmup,
                        ) {
                            tracing::error!(
                                error = ?err,
                                shader = %shader_source.display(),
                                "failed to swap shader"
                            );
                        } else {
                            surface.schedule_next_frame(qh);
                        }

                        self.surfaces.insert(surface_id, surface);
                    }
                }
            }
            WallpaperCommand::QuerySurfaces { responder } => {
                let _ = responder.send(self.collect_surface_info());
            }
            WallpaperCommand::Shutdown => {
                self.request_exit();
            }
        }
    }

    fn collect_surface_info(&self) -> Vec<SurfaceInfo> {
        self.surfaces
            .iter()
            .map(|(surface_id, surface)| {
                let output_name = surface.output_key.and_then(|key| {
                    self.output_state
                        .outputs()
                        .find(|candidate| proxy_key(candidate) == key)
                        .and_then(|matched| self.output_state.info(&matched))
                        .and_then(|info| info.name)
                });

                SurfaceInfo {
                    surface_id: *surface_id,
                    output_id: surface.output_key,
                    output_name,
                    size: surface
                        .last_output_size
                        .map(|size| (size.width, size.height)),
                }
            })
            .collect()
    }

    fn target_surface_ids(&self, selector: &SurfaceSelector) -> Vec<SurfaceId> {
        self.surfaces
            .iter()
            .filter_map(|(surface_id, surface)| {
                let matches = match selector {
                    SurfaceSelector::All => true,
                    SurfaceSelector::Surface(id) => surface_id == id,
                    SurfaceSelector::Output(id) => surface.output_key == Some(*id),
                };
                if matches {
                    Some(*surface_id)
                } else {
                    None
                }
            })
            .collect()
    }
}

impl CompositorHandler for WallpaperManager {
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
        let key = surface_key(surface);
        if let Some(mut surface_state) = self.surfaces.remove(&key) {
            if surface_state.pacer.should_render() {
                if let Err(err) = surface_state.render() {
                    surface_state.handle_render_error(err, conn, &self.compositor);
                }
            } else {
                surface_state.commit_surface();
            }

            surface_state.schedule_next_frame(qh);
            self.surfaces.insert(key, surface_state);
        }
    }
}

impl LayerShellHandler for WallpaperManager {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, layer: &LayerSurface) {
        let key = surface_key(layer.wl_surface());
        self.surfaces.remove(&key);
        if self.surfaces.is_empty() {
            self.should_exit = true;
        }
    }

    fn configure(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        let key = surface_key(layer.wl_surface());
        if let Some(mut surface_state) = self.surfaces.remove(&key) {
            let output_size = if configure.new_size.0 > 0 && configure.new_size.1 > 0 {
                Some(PhysicalSize::new(
                    configure.new_size.0,
                    configure.new_size.1,
                ))
            } else {
                surface_state.last_output_size
            };
            let resolved_size = self.resolve_configure_size(output_size);
            surface_state
                .layer_surface
                .set_size(resolved_size.width, resolved_size.height);
            surface_state.last_output_size = Some(resolved_size);
            tracing::info!(
                "layer configure new_size={}x{} -> using {}x{}",
                configure.new_size.0,
                configure.new_size.1,
                resolved_size.width,
                resolved_size.height
            );

            if let Err(err) = surface_state.ensure_gpu(conn, &self.compositor, resolved_size) {
                tracing::error!(error = ?err, "failed to prepare GPU for wallpaper");
                self.should_exit = true;
                self.surfaces.insert(key, surface_state);
                return;
            }

            if let Err(err) = surface_state.render() {
                surface_state.handle_render_error(err, conn, &self.compositor);
            }
            surface_state.schedule_next_frame(qh);
            self.surfaces.insert(key, surface_state);
        }
    }
}

impl OutputHandler for WallpaperManager {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        if let Err(err) = self.ensure_surface_for_output(conn, qh, Some(output.clone())) {
            tracing::error!(error = ?err, "failed to create surface for new output");
        }
    }

    fn update_output(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        if let Err(err) = self.ensure_surface_for_output(conn, qh, Some(output.clone())) {
            tracing::error!(error = ?err, "failed to update surface for output");
        }
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        let key = proxy_key(&output);
        self.surfaces
            .retain(|_, surface| surface.output_key != Some(key));
    }
}

impl ProvidesRegistryState for WallpaperManager {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers![OutputState];
}

delegate_compositor!(WallpaperManager);
delegate_output!(WallpaperManager);
delegate_layer!(WallpaperManager);
delegate_registry!(WallpaperManager);

struct SurfaceState {
    layer_surface: LayerSurface,
    gpu: Option<GpuState>,
    last_output_size: Option<PhysicalSize<u32>>,
    pacer: FramePacer,
    shader_source: PathBuf,
    channel_bindings: ChannelBindings,
    #[allow(dead_code)]
    crossfade: Duration,
    output_key: Option<OutputId>,
    antialiasing: Antialiasing,
    surface_alpha: SurfaceAlpha,
    shader_compiler: ShaderCompiler,
}

impl SurfaceState {
    #[allow(clippy::too_many_arguments)]
    fn new(
        layer_surface: LayerSurface,
        shader_source: PathBuf,
        channel_bindings: ChannelBindings,
        target_fps: Option<f32>,
        output_key: Option<OutputId>,
        last_output_size: Option<PhysicalSize<u32>>,
        antialiasing: Antialiasing,
        surface_alpha: SurfaceAlpha,
        shader_compiler: ShaderCompiler,
    ) -> Self {
        Self {
            layer_surface,
            gpu: None,
            last_output_size,
            pacer: FramePacer::new(target_fps),
            shader_source,
            channel_bindings,
            crossfade: Duration::from_secs_f32(1.0),
            output_key,
            antialiasing,
            surface_alpha,
            shader_compiler,
        }
    }

    fn ensure_gpu(
        &mut self,
        conn: &Connection,
        compositor: &CompositorState,
        size: PhysicalSize<u32>,
    ) -> Result<()> {
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.resize(size);
            self.apply_surface_alpha(compositor, size);
            return Ok(());
        }

        let handle = WaylandSurfaceHandle::new(conn, &self.layer_surface);
        let gpu = GpuState::new(
            &handle,
            size,
            self.shader_source.as_path(),
            &self.channel_bindings,
            self.antialiasing,
            self.shader_compiler,
        )?;
        self.apply_surface_alpha(compositor, size);
        self.pacer.reset();
        self.gpu = Some(gpu);
        Ok(())
    }

    #[allow(dead_code)]
    fn set_shader(
        &mut self,
        now: Instant,
        shader_source: &Path,
        channel_bindings: &ChannelBindings,
        crossfade: Duration,
        warmup: Duration,
    ) -> Result<()> {
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.set_shader(shader_source, channel_bindings, crossfade, warmup, now)?;
        }
        self.shader_source = shader_source.to_path_buf();
        self.channel_bindings = channel_bindings.clone();
        self.crossfade = crossfade;
        Ok(())
    }

    fn apply_render_preferences(
        &mut self,
        target_fps: Option<f32>,
        antialiasing: Antialiasing,
        surface_alpha: SurfaceAlpha,
    ) {
        self.pacer.set_target_fps(target_fps);
        if self.antialiasing != antialiasing {
            self.antialiasing = antialiasing;
            self.gpu = None;
        }
        if self.surface_alpha != surface_alpha {
            self.surface_alpha = surface_alpha;
        }
    }

    fn render(&mut self) -> Result<(), SurfaceError> {
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.render([0.0; 4])?;
            self.layer_surface.commit();
        }
        Ok(())
    }

    fn handle_render_error(
        &mut self,
        error: SurfaceError,
        conn: &Connection,
        compositor: &CompositorState,
    ) {
        self.pacer.is_frame_scheduled = false;
        match error {
            wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated => {
                self.gpu = None;
                if let Some(size) = self.last_output_size {
                    let _ = self.ensure_gpu(conn, compositor, size);
                }
            }
            wgpu::SurfaceError::OutOfMemory => {
                eprintln!("surface out of memory; dropping GPU state");
                self.gpu = None;
            }
            wgpu::SurfaceError::Timeout => {
                tracing::warn!("surface timeout; will retry next frame");
            }
            wgpu::SurfaceError::Other => {
                tracing::warn!("surface reported an unknown error; will retry next frame");
            }
        }
    }

    fn schedule_next_frame(&mut self, qh: &QueueHandle<WallpaperManager>) {
        if self.gpu.is_none() {
            return;
        }
        if self.pacer.is_frame_scheduled {
            return;
        }
        let surface = self.layer_surface.wl_surface();
        surface.frame(qh, surface.clone());
        self.layer_surface.commit();
        self.pacer.is_frame_scheduled = true;
    }

    fn commit_surface(&self) {
        self.layer_surface.commit();
    }

    fn apply_surface_alpha(&self, compositor: &CompositorState, size: PhysicalSize<u32>) {
        let surface = self.layer_surface.wl_surface();
        match self.surface_alpha {
            SurfaceAlpha::Opaque => {
                if size.width == 0 || size.height == 0 {
                    surface.set_opaque_region(None);
                    return;
                }
                let width = size.width.min(i32::MAX as u32) as i32;
                let height = size.height.min(i32::MAX as u32) as i32;
                match Region::new(compositor) {
                    Ok(region) => {
                        region.add(0, 0, width, height);
                        surface.set_opaque_region(Some(region.wl_region()));
                    }
                    Err(err) => {
                        tracing::warn!(
                            error = %err,
                            "failed to declare opaque region for wallpaper surface"
                        );
                        surface.set_opaque_region(None);
                    }
                }
            }
            SurfaceAlpha::Transparent => {
                surface.set_opaque_region(None);
            }
        }
    }
}

struct FramePacer {
    target_interval: Option<Duration>,
    accumulator: Duration,
    last_tick: Option<Instant>,
    is_frame_scheduled: bool,
}

impl FramePacer {
    fn new(target_fps: Option<f32>) -> Self {
        let target_interval = target_fps.and_then(|fps| {
            if fps > 0.0 {
                Some(Duration::from_secs_f32(1.0 / fps))
            } else {
                None
            }
        });
        Self {
            target_interval,
            accumulator: Duration::ZERO,
            last_tick: None,
            is_frame_scheduled: false,
        }
    }

    fn set_target_fps(&mut self, target_fps: Option<f32>) {
        self.target_interval = target_fps.and_then(|fps| {
            if fps > 0.0 {
                Some(Duration::from_secs_f32(1.0 / fps))
            } else {
                None
            }
        });
        self.accumulator = Duration::ZERO;
        self.last_tick = None;
        self.is_frame_scheduled = false;
    }

    fn reset(&mut self) {
        self.accumulator = Duration::ZERO;
        self.last_tick = Some(Instant::now());
        self.is_frame_scheduled = false;
    }

    fn should_render(&mut self) -> bool {
        let now = Instant::now();
        match (self.target_interval, self.last_tick) {
            (Some(interval), Some(last)) => {
                let delta = now.saturating_duration_since(last);
                self.last_tick = Some(now);
                self.accumulator = self.accumulator.saturating_add(delta);
                if self.accumulator + Duration::from_micros(250) < interval {
                    self.is_frame_scheduled = false;
                    false
                } else {
                    self.accumulator = self.accumulator.saturating_sub(interval);
                    self.is_frame_scheduled = false;
                    true
                }
            }
            (Some(_), None) => {
                self.last_tick = Some(now);
                self.is_frame_scheduled = false;
                true
            }
            (None, _) => {
                self.last_tick = Some(now);
                self.is_frame_scheduled = false;
                true
            }
        }
    }
}

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

impl raw_window_handle::HasDisplayHandle for WaylandSurfaceHandle {
    fn display_handle(
        &self,
    ) -> StdResult<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError> {
        let display =
            NonNull::new(self.display).ok_or(raw_window_handle::HandleError::Unavailable)?;
        let wayland = raw_window_handle::WaylandDisplayHandle::new(display);
        let raw = raw_window_handle::RawDisplayHandle::Wayland(wayland);
        Ok(unsafe { raw_window_handle::DisplayHandle::borrow_raw(raw) })
    }
}

impl raw_window_handle::HasWindowHandle for WaylandSurfaceHandle {
    fn window_handle(
        &self,
    ) -> StdResult<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
        let surface =
            NonNull::new(self.surface).ok_or(raw_window_handle::HandleError::Unavailable)?;
        let wayland = raw_window_handle::WaylandWindowHandle::new(surface);
        let raw = raw_window_handle::RawWindowHandle::Wayland(wayland);
        Ok(unsafe { raw_window_handle::WindowHandle::borrow_raw(raw) })
    }
}

fn proxy_key<P: Proxy>(proxy: &P) -> OutputId {
    OutputId(proxy.id().as_ptr() as u64)
}

fn surface_key(surface: &wl_surface::WlSurface) -> SurfaceId {
    SurfaceId(surface.id().as_ptr() as u64)
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
