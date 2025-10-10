//! Wayland wallpaper runtime (layer-shell surfaces, multi-output management, pacing).
//!
//! This module integrates with Wayland via smithay-client-toolkit to create background
//! layer surfaces per output and render ShaderToy shaders as wallpapers. It handles
//! output discovery/updates, layer configure events, frame callbacks, and orchestrates
//! GPU state and shader swaps with smooth crossfades and warmup frames.
//!
//! Architecture
//!
//! ```text
//! WallpaperRuntime
//!   ├─ channel (Sender<WallpaperCommand>)  ◀────────── wax11 daemon/CLI
//!   └─ thread: run_internal
//!        ├─ Wayland registry + event queue
//!        ├─ WallpaperManager
//!        │    ├─ surfaces: { SurfaceId → SurfaceState }
//!        │    ├─ handle_command(Swap/Query/Shutdown)
//!        │    └─ delegate_* handlers (layer/output/registry)
//!        └─ loop: drain commands → dispatch → frame callbacks
//!
//! SurfaceState lifecycle
//!
//! - Creates a `GpuState` bound to a wl_surface via a temporary `WaylandSurfaceHandle`.
//! - Applies `SurfaceAlpha` by setting wl_region opaque rectangles when appropriate.
//! - Uses `FramePacer` to honour FPS caps (including software rasterizer hints).
//! - Renders on `configure`/frame callbacks and schedules the next frame.
//! - Supports shader swaps with optional warmup and crossfade; rebuilds GPU state when
//!   channel layout or core format preferences change.
//!
//! Coordination with other modules
//!
//! - `types::RendererConfig` seeds initial preferences; future swaps arrive via commands.
//! - `gpu::GpuState` performs rendering; `runtime::time_source_for_policy` provides time.
//! - Export/still policies render once and optionally trigger process exit when all
//!   surfaces complete (mirrors preview’s one-shot export behaviour).
//!
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

use crate::gpu::{FileExportTarget, GpuState, RenderExportError};
use crate::runtime::{time_source_for_policy, BoxedTimeSource, FillMethod, RenderPolicy};
use crate::types::{
    AdapterProfile, Antialiasing, ChannelBindings, ColorSpaceMode, GpuMemoryMode,
    GpuPowerPreference, RendererConfig, ShaderCompiler, SurfaceAlpha,
};

const SOFTWARE_FPS_CAP: f32 = 15.0;

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
    pub color_space: ColorSpaceMode,
    pub warmup: Duration,
    pub policy: RenderPolicy,
}

impl WallpaperRuntime {
    pub fn spawn(config: RendererConfig) -> Result<Self> {
        let (sender, receiver) = crossbeam_channel::unbounded();
        let handle = thread::Builder::new()
            .name("wax11-wallpaper".into())
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
            .send(WallpaperCommand::SwapShader {
                selector,
                request: Box::new(request),
            })
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
        request: Box<SwapRequest>,
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
    )?;
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
    color_space: ColorSpaceMode,
    shader_compiler: ShaderCompiler,
    should_exit: bool,
    render_scale: f32,
    fill_method: FillMethod,
    software_hint_emitted: bool,
    base_policy: RenderPolicy,
    exit_after_export: bool,
    gpu_power: GpuPowerPreference,
    gpu_memory: GpuMemoryMode,
    gpu_latency: u32,
}

impl WallpaperManager {
    fn new(
        compositor: CompositorState,
        layer_shell: LayerShell,
        registry_state: RegistryState,
        output_state: OutputState,
        config: &RendererConfig,
    ) -> Result<Self> {
        Ok(Self {
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
            color_space: config.color_space,
            shader_compiler: config.shader_compiler,
            should_exit: false,
            render_scale: config.render_scale,
            fill_method: config.fill_method,
            software_hint_emitted: false,
            base_policy: config.policy.clone(),
            exit_after_export: config.exit_on_export,
            gpu_power: config.gpu_power,
            gpu_memory: config.gpu_memory,
            gpu_latency: config.gpu_latency,
        })
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

    fn register_export_completion(&mut self, policy: &RenderPolicy) {
        if !self.exit_after_export {
            return;
        }
        if !matches!(self.base_policy, RenderPolicy::Export { .. }) {
            return;
        }
        if matches!(policy, RenderPolicy::Export { .. }) {
            self.should_exit = true;
        }
    }

    fn maybe_exit_after_export(&mut self) {
        if !self.exit_after_export {
            return;
        }
        if !matches!(self.base_policy, RenderPolicy::Export { .. }) {
            return;
        }
        if self.surfaces.values().all(|surface| surface.is_rendered()) {
            self.should_exit = true;
        }
    }

    fn log_software_cap_if_needed(&mut self, profile: &AdapterProfile) {
        if self.software_hint_emitted {
            return;
        }
        tracing::warn!(
            adapter = %profile.name,
            backend = ?profile.backend,
            cap = SOFTWARE_FPS_CAP,
            "software rasterizer detected; capping wallpaper FPS to {} FPS (override with --fps)",
            SOFTWARE_FPS_CAP
        );
        self.software_hint_emitted = true;
    }

    fn ensure_surface_for_output(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        output: Option<wl_output::WlOutput>,
    ) -> Result<()> {
        if let Some(ref out) = output {
            let key = proxy_key(out);
            if let Some(surface_id) = self
                .surfaces
                .iter()
                .find(|(_, surface)| surface.output_key == Some(key))
                .map(|(surface_id, _)| *surface_id)
            {
                let mut profile_to_log: Option<AdapterProfile> = None;
                if let Some(existing) = self.surfaces.get_mut(&surface_id) {
                    existing.last_output_size = output
                        .as_ref()
                        .and_then(|o| self.output_state.info(o))
                        .and_then(output_info_physical_size);
                    if let Some(size) = existing.last_output_size {
                        if existing.ensure_gpu(conn, &self.compositor, size).is_ok()
                            && existing.software_cap_applied()
                        {
                            if let Some(profile) = existing.adapter_profile() {
                                profile_to_log = Some(profile.clone());
                            }
                        }
                    }
                }
                if let Some(profile) = profile_to_log {
                    self.log_software_cap_if_needed(&profile);
                }
                return Ok(());
            }
        }

        let wl_surface = self.compositor.create_surface(qh);
        let layer_surface = self.layer_shell.create_layer_surface(
            qh,
            wl_surface,
            Layer::Background,
            Some("wax11".to_string()),
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
        let mut surface_state = SurfaceState::new(
            layer_surface,
            self.shader_source.clone(),
            self.channel_bindings.clone(),
            self.target_fps,
            output_key,
            initial_size,
            self.antialiasing,
            self.surface_alpha,
            self.color_space,
            self.shader_compiler,
            self.render_scale,
            self.fill_method,
            self.base_policy.clone(),
            self.gpu_power,
            self.gpu_memory,
            self.gpu_latency,
        )?;
        if let Some(size) = initial_size {
            if surface_state
                .ensure_gpu(conn, &self.compositor, size)
                .is_ok()
                && surface_state.software_cap_applied()
            {
                if let Some(profile) = surface_state.adapter_profile() {
                    self.log_software_cap_if_needed(profile);
                }
            }
            if matches!(surface_state.policy, RenderPolicy::Export { .. }) {
                if let Err(err) = surface_state.render() {
                    surface_state.handle_render_error(err, conn, &self.compositor);
                } else if surface_state.mark_rendered() {
                    self.register_export_completion(&surface_state.policy);
                }
            }
        }
        self.surfaces.insert(key, surface_state);

        self.maybe_exit_after_export();
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
                    mut crossfade,
                    target_fps,
                    antialiasing,
                    surface_alpha,
                    color_space,
                    warmup,
                    policy,
                } = *request;
                let now = Instant::now();
                for surface_id in self.target_surface_ids(&selector) {
                    if let Some(mut surface) = self.surfaces.remove(&surface_id) {
                        let layout_signature = channel_bindings.layout_signature();
                        let requires_gpu_rebuild = surface.antialiasing != antialiasing
                            || surface.color_space != color_space
                            || surface
                                .gpu
                                .as_ref()
                                .map(|gpu| gpu.channel_kinds() != &layout_signature)
                                .unwrap_or(false);

                        if requires_gpu_rebuild && crossfade > Duration::ZERO {
                            tracing::info!(
                                target = %surface_id.0,
                                crossfade_ms = crossfade.as_millis(),
                                old_antialias = ?surface.antialiasing,
                                new_antialias = ?antialiasing,
                                old_color = ?surface.color_space,
                                new_color = ?color_space,
                                "crossfade disabled: shader swap requires fresh GPU state"
                            );
                            crossfade = Duration::ZERO;
                        }
                        surface.apply_render_preferences(
                            target_fps,
                            antialiasing,
                            surface_alpha,
                            color_space,
                        );
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
                            if let Err(err) = surface.set_policy(policy.clone()) {
                                tracing::error!(error = %err, "failed to update render policy");
                            }
                            if matches!(surface.policy, RenderPolicy::Export { .. }) {
                                if let Err(err) = surface.render() {
                                    surface.handle_render_error(err, conn, &self.compositor);
                                } else if surface.mark_rendered() {
                                    self.register_export_completion(&surface.policy);
                                }
                            } else {
                                surface.schedule_next_frame(qh);
                            }
                        }

                        self.surfaces.insert(surface_id, surface);
                        self.maybe_exit_after_export();
                    }
                }
                self.color_space = color_space;
                self.target_fps = target_fps;
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
            tracing::trace!(policy = ?surface_state.policy, rendered = surface_state.rendered_once, "frame callback");
            if surface_state.should_render() {
                match surface_state.render() {
                    Ok(()) => {
                        if surface_state.mark_rendered() {
                            self.register_export_completion(&surface_state.policy);
                        }
                    }
                    Err(err) => {
                        surface_state.handle_render_error(err, conn, &self.compositor);
                    }
                }
            } else {
                surface_state.commit_surface();
            }

            surface_state.schedule_next_frame(qh);
            self.surfaces.insert(key, surface_state);
            self.maybe_exit_after_export();
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
            tracing::debug!(
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

            if surface_state.software_cap_applied() {
                if let Some(profile) = surface_state.adapter_profile() {
                    self.log_software_cap_if_needed(profile);
                }
            }

            if let Err(err) = surface_state.render() {
                surface_state.handle_render_error(err, conn, &self.compositor);
            } else if surface_state.mark_rendered() {
                self.register_export_completion(&surface_state.policy);
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
    rendered_once: bool,
    shader_source: PathBuf,
    channel_bindings: ChannelBindings,
    crossfade: Duration,
    output_key: Option<OutputId>,
    antialiasing: Antialiasing,
    surface_alpha: SurfaceAlpha,
    color_space: ColorSpaceMode,
    shader_compiler: ShaderCompiler,
    render_scale: f32,
    fill_method: FillMethod,
    requested_target_fps: Option<f32>,
    software_cap_applied: bool,
    policy: RenderPolicy,
    time_source: BoxedTimeSource,
    gpu_power: GpuPowerPreference,
    gpu_memory: GpuMemoryMode,
    gpu_latency: u32,
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
        color_space: ColorSpaceMode,
        shader_compiler: ShaderCompiler,
        render_scale: f32,
        fill_method: FillMethod,
        policy: RenderPolicy,
        gpu_power: GpuPowerPreference,
        gpu_memory: GpuMemoryMode,
        gpu_latency: u32,
    ) -> Result<Self> {
        let time_source = time_source_for_policy(&policy)?;
        Ok(Self {
            layer_surface,
            gpu: None,
            last_output_size,
            pacer: FramePacer::new(target_fps),
            rendered_once: false,
            shader_source,
            channel_bindings,
            crossfade: Duration::from_secs_f32(1.0),
            output_key,
            antialiasing,
            surface_alpha,
            color_space,
            shader_compiler,
            render_scale,
            fill_method,
            requested_target_fps: target_fps,
            software_cap_applied: false,
            policy,
            time_source,
            gpu_power,
            gpu_memory,
            gpu_latency,
        })
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
            self.color_space,
            self.shader_compiler,
            self.render_scale,
            self.fill_method,
            self.gpu_power,
            self.gpu_memory,
            self.gpu_latency,
        )?;
        let is_software = gpu.adapter_profile().is_software();
        if is_software {
            if self.requested_target_fps.is_none() {
                self.pacer.set_target_fps(Some(SOFTWARE_FPS_CAP));
                self.requested_target_fps = Some(SOFTWARE_FPS_CAP);
            }
            self.software_cap_applied = self.requested_target_fps == Some(SOFTWARE_FPS_CAP);
        } else {
            self.software_cap_applied = false;
        }
        self.apply_surface_alpha(compositor, size);
        self.reset_render_state();
        self.gpu = Some(gpu);
        Ok(())
    }

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
        self.rendered_once = false;
        Ok(())
    }

    fn apply_render_preferences(
        &mut self,
        target_fps: Option<f32>,
        antialiasing: Antialiasing,
        surface_alpha: SurfaceAlpha,
        color_space: ColorSpaceMode,
    ) {
        self.pacer.set_target_fps(target_fps);
        self.requested_target_fps = target_fps;
        if target_fps.is_some() {
            self.software_cap_applied = false;
        }
        if self.antialiasing != antialiasing {
            self.antialiasing = antialiasing;
            self.gpu = None;
        }
        if self.surface_alpha != surface_alpha {
            self.surface_alpha = surface_alpha;
        }
        if self.color_space != color_space {
            self.color_space = color_space;
            self.gpu = None;
        }
        self.rendered_once = false;
    }

    fn adapter_profile(&self) -> Option<&AdapterProfile> {
        self.gpu.as_ref().map(|gpu| gpu.adapter_profile())
    }

    fn software_cap_applied(&self) -> bool {
        self.software_cap_applied
    }

    fn reset_render_state(&mut self) {
        self.rendered_once = false;
        self.pacer.reset();
    }

    fn set_policy(&mut self, policy: RenderPolicy) -> Result<()> {
        let previous = self.policy.clone();
        let preserve_time = matches!(
            (&previous, &policy),
            (RenderPolicy::Animate { .. }, RenderPolicy::Animate { .. })
        );

        self.policy = policy.clone();
        if !preserve_time {
            self.time_source = time_source_for_policy(&self.policy)?;
        }
        self.reset_render_state();
        Ok(())
    }

    fn is_rendered(&self) -> bool {
        self.rendered_once
    }

    fn should_render(&mut self) -> bool {
        match self.policy {
            RenderPolicy::Still { .. } | RenderPolicy::Export { .. } => !self.rendered_once,
            _ => self.pacer.should_render(),
        }
    }

    fn mark_rendered(&mut self) -> bool {
        if matches!(
            self.policy,
            RenderPolicy::Still { .. } | RenderPolicy::Export { .. }
        ) {
            let first_render = !self.rendered_once;
            self.rendered_once = true;
            first_render
        } else {
            false
        }
    }

    fn render(&mut self) -> Result<(), SurfaceError> {
        if let Some(gpu) = self.gpu.as_mut() {
            let sample = self.time_source.sample();
            let export_result = match &self.policy {
                RenderPolicy::Export { path, format, .. } => {
                    let target = FileExportTarget {
                        path: path.clone(),
                        format: *format,
                    };
                    Some(gpu.render_export([0.0; 4], Some(sample), &target))
                }
                _ => {
                    gpu.render([0.0; 4], Some(sample))?;
                    None
                }
            };

            if let Some(result) = export_result {
                match result {
                    Ok(_) => {}
                    Err(RenderExportError::Surface(surface_err)) => return Err(surface_err),
                    Err(RenderExportError::Export(other)) => {
                        tracing::error!(error = %other, "failed to export still frame");
                    }
                }
            }

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
        if matches!(
            self.policy,
            RenderPolicy::Still { .. } | RenderPolicy::Export { .. }
        ) && self.rendered_once
        {
            self.pacer.is_frame_scheduled = false;
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
