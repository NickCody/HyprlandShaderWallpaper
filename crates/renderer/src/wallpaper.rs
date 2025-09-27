use std::ffi::c_void;
use std::path::PathBuf;
use std::ptr::NonNull;
use std::result::Result as StdResult;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use smithay_client_toolkit::reexports::client::{
    globals::registry_queue_init,
    protocol::{wl_output, wl_surface},
    Connection, Proxy, QueueHandle,
};
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
    shell::WaylandSurface,
};
use tracing::info;
use winit::dpi::PhysicalSize;

use crate::gpu::GpuState;
use crate::types::{Antialiasing, ChannelBindings, RendererConfig, SurfaceAlpha};

pub(crate) fn run(config: &RendererConfig) -> Result<()> {
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
    let mut manager = WallpaperManager::new(
        compositor,
        registry_state,
        output_state,
        layer_surface,
        config.shader_source.clone(),
        config.requested_size,
        fallback_size,
        config.channel_bindings.clone(),
        config.antialiasing,
        config.surface_alpha,
        config.target_fps,
        target_output,
        initial_output_size,
    );

    loop {
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
    registry_state: RegistryState,
    output_state: OutputState,
    compositor: CompositorState,
    surface: SurfaceState,
    shader_source: PathBuf,
    requested_size: Option<(u32, u32)>,
    fallback_size: PhysicalSize<u32>,
    channel_bindings: ChannelBindings,
    antialiasing: Antialiasing,
    surface_alpha: SurfaceAlpha,
    target_output: Option<wl_output::WlOutput>,
    should_exit: bool,
}

impl WallpaperManager {
    fn new(
        compositor: CompositorState,
        registry_state: RegistryState,
        output_state: OutputState,
        layer_surface: LayerSurface,
        shader_source: PathBuf,
        requested_size: Option<(u32, u32)>,
        fallback_size: PhysicalSize<u32>,
        channel_bindings: ChannelBindings,
        antialiasing: Antialiasing,
        surface_alpha: SurfaceAlpha,
        target_fps: Option<f32>,
        target_output: Option<wl_output::WlOutput>,
        initial_output_size: Option<PhysicalSize<u32>>,
    ) -> Self {
        let surface = SurfaceState::new(layer_surface, target_fps, initial_output_size);
        Self {
            registry_state,
            output_state,
            compositor,
            surface,
            shader_source,
            requested_size,
            fallback_size,
            channel_bindings,
            antialiasing,
            surface_alpha,
            target_output,
            should_exit: false,
        }
    }

    fn should_exit(&self) -> bool {
        self.should_exit
    }

    fn infer_output_size(&self) -> Option<PhysicalSize<u32>> {
        if let Some(output) = self.target_output.as_ref() {
            if let Some(info) = self.output_state.info(output) {
                return output_info_physical_size(info);
            }
        }
        self.surface.last_output_size
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
                self.surface.last_output_size = physical;
            } else if self.target_output.is_none() {
                self.target_output = Some(output.clone());
                self.surface.last_output_size = physical;
            }
        }
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
        if !self.surface.matches(surface) {
            return;
        }

        self.surface.frame_scheduled = false;

        if let Some(gpu) = self.surface.gpu.as_mut() {
            if self.surface.pacer.should_render() {
                tracing::trace!("frame callback - rendering");
                match gpu.render_frame([0.0; 4]) {
                    Ok(()) => {}
                    Err(err) => self.surface.handle_surface_error(err),
                }
            } else {
                tracing::trace!("frame callback - skipped render due to fps cap");
            }
            self.surface.layer_surface.commit();
        } else if let Some(size) = self.infer_output_size() {
            let created = match self.surface.ensure_gpu(
                conn,
                size,
                self.shader_source.as_path(),
                &self.channel_bindings,
                self.antialiasing,
            ) {
                Ok(created) => created,
                Err(err) => {
                    eprintln!("failed to initialize GPU for wallpaper: {err:?}");
                    self.should_exit = true;
                    return;
                }
            };

            if created {
                if let Some(gpu) = self.surface.gpu.as_mut() {
                    tracing::trace!("rendering bootstrap frame after missing gpu");
                    if let Err(err) = gpu.render_frame([0.0; 4]) {
                        self.surface.handle_surface_error(err);
                        return;
                    }
                    self.surface
                        .apply_surface_alpha(&self.compositor, self.surface_alpha, size);
                    self.surface.layer_surface.commit();
                    tracing::trace!("committed bootstrap frame");
                }
            }
        }

        self.surface.schedule_frame(qh);
    }
}

impl LayerShellHandler for WallpaperManager {
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
        self.surface.layer_surface.set_size(size.width, size.height);
        self.surface.last_output_size = Some(size);
        info!(
            "layer configure new_size={}x{} -> using {}x{}",
            configure.new_size.0, configure.new_size.1, size.width, size.height
        );

        let created = match self.surface.ensure_gpu(
            conn,
            size,
            self.shader_source.as_path(),
            &self.channel_bindings,
            self.antialiasing,
        ) {
            Ok(created) => created,
            Err(err) => {
                eprintln!("failed to prepare GPU for wallpaper: {err:?}");
                self.should_exit = true;
                return;
            }
        };

        if created {
            if let Some(gpu) = self.surface.gpu.as_mut() {
                tracing::trace!("rendering bootstrap frame");
                if let Err(err) = gpu.render_frame([0.0; 4]) {
                    self.surface.handle_surface_error(err);
                    return;
                }
                self.surface.layer_surface.commit();
                tracing::trace!("committed bootstrap frame");
            }
        }

        self.surface
            .apply_surface_alpha(&self.compositor, self.surface_alpha, size);
        self.surface.schedule_frame(qh);
    }
}

impl OutputHandler for WallpaperManager {
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
            self.surface.last_output_size = None;
        }
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
    frame_scheduled: bool,
    last_output_size: Option<PhysicalSize<u32>>,
    pacer: FramePacer,
}

impl SurfaceState {
    fn new(
        layer_surface: LayerSurface,
        target_fps: Option<f32>,
        last_output_size: Option<PhysicalSize<u32>>,
    ) -> Self {
        Self {
            layer_surface,
            gpu: None,
            frame_scheduled: false,
            last_output_size,
            pacer: FramePacer::new(target_fps),
        }
    }

    fn matches(&self, surface: &wl_surface::WlSurface) -> bool {
        surface == self.layer_surface.wl_surface()
    }

    fn ensure_gpu(
        &mut self,
        conn: &Connection,
        size: PhysicalSize<u32>,
        shader_source: &std::path::Path,
        channel_bindings: &ChannelBindings,
        antialiasing: Antialiasing,
    ) -> Result<bool> {
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.resize(size);
            tracing::debug!("resized GPU surface to {}x{}", size.width, size.height);
            return Ok(false);
        }

        let handle = WaylandSurfaceHandle::new(conn, &self.layer_surface);
        let gpu = GpuState::new(&handle, size, shader_source, channel_bindings, antialiasing)?;
        tracing::info!("initialised GPU surface {}x{}", size.width, size.height);
        self.gpu = Some(gpu);
        self.pacer.reset();
        Ok(true)
    }

    fn schedule_frame(&mut self, qh: &QueueHandle<WallpaperManager>) {
        if self.frame_scheduled || self.gpu.is_none() {
            return;
        }
        let surface = self.layer_surface.wl_surface();
        surface.frame(qh, surface.clone());
        self.frame_scheduled = true;
        self.layer_surface.commit();
        tracing::trace!("requested frame callback and committed surface");
    }

    fn apply_surface_alpha(
        &self,
        compositor: &CompositorState,
        surface_alpha: SurfaceAlpha,
        size: PhysicalSize<u32>,
    ) {
        let surface = self.layer_surface.wl_surface();
        match surface_alpha {
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

    fn handle_surface_error(&mut self, error: wgpu::SurfaceError) {
        match error {
            wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated => {
                if let Some(gpu) = self.gpu.as_mut() {
                    gpu.resize(gpu.size());
                }
            }
            wgpu::SurfaceError::OutOfMemory => {
                eprintln!("surface out of memory; exiting wallpaper loop");
                self.gpu = None;
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

struct FramePacer {
    target_interval: Option<Duration>,
    accumulator: Duration,
    last_tick: Option<Instant>,
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
        }
    }

    fn reset(&mut self) {
        self.accumulator = Duration::ZERO;
        self.last_tick = Some(Instant::now());
    }

    fn should_render(&mut self) -> bool {
        let now = Instant::now();
        match (self.target_interval, self.last_tick) {
            (Some(interval), Some(last)) => {
                let delta = now.saturating_duration_since(last);
                self.last_tick = Some(now);
                self.accumulator = self.accumulator.saturating_add(delta);
                if self.accumulator + Duration::from_micros(250) < interval {
                    false
                } else {
                    self.accumulator = self.accumulator.saturating_sub(interval);
                    true
                }
            }
            (Some(_), None) => {
                self.last_tick = Some(now);
                true
            }
            (None, _) => {
                self.last_tick = Some(now);
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
