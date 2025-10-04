use std::sync::Arc;

use std::path::{Path, PathBuf};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use crossbeam_channel::{bounded, unbounded, Receiver, Sender};
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, Event, KeyEvent, WindowEvent};
use winit::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowBuilder};

use tracing::{error, info};

use crate::gpu::{FileExportTarget, GpuState, RenderExportError};
use crate::runtime::{
    time_source_for_policy, BoxedTimeSource, FillMethod, FrameScheduler, RenderPolicy, TimeSample,
};
use crate::types::{
    AdapterProfile, Antialiasing, ChannelBindings, ColorSpaceMode, RendererConfig, ShaderCompiler,
    SurfaceAlpha,
};
use crate::wallpaper::SwapRequest;

const SOFTWARE_FPS_CAP: f32 = 15.0;

/// Aggregates GPU state for the windowed preview path.
pub(crate) struct WindowState {
    window: Arc<Window>,
    gpu: Option<GpuState>,
    mouse: MouseState,
    keyboard: KeyboardState,
    antialiasing: Antialiasing,
    shader_compiler: ShaderCompiler,
    color_space: ColorSpaceMode,
    render_scale: f32,
    fill_method: FillMethod,
    frame_sink: FrameSinkDriver,
    surface_alpha: SurfaceAlpha,
}

#[derive(Debug)]
enum FrameSinkDriver {
    Surface,
    Export(FileExportState),
}

#[derive(Debug)]
struct FileExportState {
    target: FileExportTarget,
    captured: bool,
}

#[derive(Debug)]
pub(crate) enum RenderFrameStatus {
    Presented,
    Captured(PathBuf),
}

impl WindowState {
    pub(crate) fn new(window: Arc<Window>, config: &RendererConfig) -> Result<Self> {
        let size = window.inner_size();
        let gpu = GpuState::new(
            window.as_ref(),
            size,
            &config.shader_source,
            &config.channel_bindings,
            config.antialiasing,
            config.color_space,
            config.shader_compiler,
            config.render_scale,
            config.fill_method,
        )?;

        let frame_sink = match &config.policy {
            RenderPolicy::Export { path, format, .. } => FrameSinkDriver::Export(FileExportState {
                target: FileExportTarget {
                    path: path.clone(),
                    format: *format,
                },
                captured: false,
            }),
            _ => FrameSinkDriver::Surface,
        };

        let mut state = Self {
            window,
            gpu: Some(gpu),
            mouse: MouseState::default(),
            keyboard: KeyboardState::default(),
            antialiasing: config.antialiasing,
            shader_compiler: config.shader_compiler,
            color_space: config.color_space,
            render_scale: config.render_scale,
            fill_method: config.fill_method,
            frame_sink,
            surface_alpha: config.surface_alpha,
        };
        state.sync_keyboard(true);
        Ok(state)
    }

    pub(crate) fn adapter_profile(&self) -> &AdapterProfile {
        self.gpu.as_ref().expect("gpu initialized").adapter_profile()
    }

    pub(crate) fn window(&self) -> &Window {
        self.window.as_ref()
    }

    pub(crate) fn size(&self) -> PhysicalSize<u32> {
        self.gpu.as_ref().expect("gpu initialized").size()
    }

    pub(crate) fn resize(&mut self, new_size: PhysicalSize<u32>) {
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.resize(new_size);
        }
    }

    pub(crate) fn render_frame(
        &mut self,
        time_sample: TimeSample,
    ) -> Result<RenderFrameStatus, RenderExportError> {
        self.sync_keyboard(false);
        let mouse_uniform = self.mouse.as_uniform(self.size().height.max(1) as f32);
        let result = match &mut self.frame_sink {
            FrameSinkDriver::Surface => self
                .gpu
                .as_mut()
                .expect("gpu initialized")
                .render(mouse_uniform, Some(time_sample))
                .map(|_| RenderFrameStatus::Presented)
                .map_err(RenderExportError::Surface),
            FrameSinkDriver::Export(state) => {
                if state.captured {
                    Ok(RenderFrameStatus::Captured(state.target.path.clone()))
                } else {
                    self
                        .gpu
                        .as_mut()
                        .expect("gpu initialized")
                        .render_export(mouse_uniform, Some(time_sample), &state.target)
                        .map(|path| {
                            state.captured = true;
                            RenderFrameStatus::Captured(path)
                        })
                }
            }
        };

        if result.is_ok() {
            self.flush_keyboard_pulses();
        }
        result
    }

    pub(crate) fn set_shader(
        &mut self,
        shader_source: &Path,
        channel_bindings: &ChannelBindings,
        antialiasing: Antialiasing,
        surface_alpha: SurfaceAlpha,
        color_space: ColorSpaceMode,
        crossfade: Duration,
        warmup: Duration,
    ) -> Result<()> {
        let preferences_changed =
            self.surface_alpha != surface_alpha || self.color_space != color_space;
        if preferences_changed {
            self.surface_alpha = surface_alpha;
            self.color_space = color_space;
        }
        let layout_signature = channel_bindings.layout_signature();
        let layout_changed = self
            .gpu
            .as_ref()
            .expect("gpu initialized")
            .channel_kinds()
            != &layout_signature;
        if self.antialiasing != antialiasing || layout_changed || preferences_changed {
            if layout_changed {
                info!("channel binding layout changed; rebuilding GPU state without crossfade");
            }
            self.antialiasing = antialiasing;
            let size = self.window.inner_size();
            // Drop the old surface/device before creating a new one to avoid
            // multiple wgpu surfaces bound to the same Wayland wl_surface.
            if let Some(old) = self.gpu.take() {
                drop(old);
            }
            let new_gpu = GpuState::new(
                self.window.as_ref(),
                size,
                shader_source,
                channel_bindings,
                antialiasing,
                self.color_space,
                self.shader_compiler,
                self.render_scale,
                self.fill_method,
            )?;
            self.gpu = Some(new_gpu);
        } else {
            self
                .gpu
                .as_mut()
                .expect("gpu initialized")
                .set_shader(
                shader_source,
                channel_bindings,
                crossfade,
                warmup,
                Instant::now(),
            )?;
        }
        self.sync_keyboard(true);
        Ok(())
    }

    pub(crate) fn handle_cursor_moved(&mut self, position: PhysicalPosition<f64>) {
        self.mouse.handle_cursor_moved(position);
    }

    pub(crate) fn handle_mouse_button(&mut self, state: ElementState) {
        self.mouse.handle_button(state);
    }

    fn sync_keyboard(&mut self, force: bool) {
        if !self
            .gpu
            .as_ref()
            .expect("gpu initialized")
            .has_keyboard_channel()
        {
            if force {
                self.keyboard.clear_dirty();
            }
            return;
        }

        if force {
            let snapshot = self.keyboard.snapshot();
            if let Some(gpu) = self.gpu.as_ref() {
                gpu.update_keyboard_channels(&snapshot);
            }
        }

        while let Some(snapshot) = self.keyboard.take_dirty_snapshot() {
            if let Some(gpu) = self.gpu.as_ref() {
                gpu.update_keyboard_channels(&snapshot);
            }
        }
    }

    fn flush_keyboard_pulses(&mut self) {
        if let Some(snapshot) = self.keyboard.take_pulse_reset_snapshot() {
            if let Some(gpu) = self.gpu.as_ref() {
                if gpu.has_keyboard_channel() {
                    gpu.update_keyboard_channels(&snapshot);
                }
            }
        }
    }
}

pub(crate) struct RenderPolicyDriver {
    scheduler: FrameScheduler,
    time_source: BoxedTimeSource,
}

impl RenderPolicyDriver {
    pub(crate) fn new(policy: RenderPolicy) -> Result<Self> {
        Ok(Self {
            scheduler: FrameScheduler::new(policy.clone()),
            time_source: time_source_for_policy(&policy)?,
        })
    }

    pub(crate) fn sample(&mut self) -> TimeSample {
        self.time_source.sample()
    }

    pub(crate) fn mark_rendered(&mut self) {
        self.scheduler.mark_rendered();
    }

    pub(crate) fn ready_for_frame(&mut self, now: Instant) -> bool {
        self.scheduler.ready_for_frame(now)
    }

    pub(crate) fn next_deadline(&self) -> Option<Instant> {
        self.scheduler.next_deadline()
    }

    pub(crate) fn reset(&mut self) {
        self.time_source.reset();
        self.scheduler.reset();
    }
}

#[derive(Debug, Clone)]
enum WindowCommand {
    Swap { request: SwapRequest },
    Shutdown,
}

#[derive(Debug, Clone)]
enum WindowSignal {
    AdvancePlaylist,
}

pub struct WindowRuntime {
    proxy: EventLoopProxy<WindowCommand>,
    events: Receiver<WindowSignal>,
    join_handle: Option<JoinHandle<Result<()>>>,
}

impl WindowRuntime {
    pub fn spawn(config: RendererConfig) -> Result<Self> {
        let (ready_tx, ready_rx) = bounded(1);
        let (signal_tx, signal_rx) = unbounded();
        let handle = thread::Builder::new()
            .name("lambdash-window".into())
            .spawn(move || run_window_thread(config, ready_tx, signal_tx))
            .map_err(|err| anyhow!("failed to spawn window thread: {err}"))?;

        let proxy = ready_rx
            .recv()
            .map_err(|err| anyhow!("window thread failed to initialise: {err}"))??;

        Ok(Self {
            proxy,
            events: signal_rx,
            join_handle: Some(handle),
        })
    }

    pub fn swap_shader(&self, request: SwapRequest) -> Result<()> {
        self.proxy
            .send_event(WindowCommand::Swap { request })
            .map_err(|err| anyhow!(err))
    }

    pub fn shutdown(mut self) -> Result<()> {
        if let Some(handle) = self.join_handle.take() {
            let _ = self.proxy.send_event(WindowCommand::Shutdown);
            handle
                .join()
                .map_err(|err| anyhow!("window thread panicked: {err:?}"))??;
        }
        Ok(())
    }

    pub fn take_advance_requests(&self) -> usize {
        self.events
            .try_iter()
            .filter(|signal| matches!(signal, WindowSignal::AdvancePlaylist))
            .count()
    }
}

impl Drop for WindowRuntime {
    fn drop(&mut self) {
        if let Some(handle) = self.join_handle.take() {
            let _ = self.proxy.send_event(WindowCommand::Shutdown);
            let _ = handle.join();
        }
    }
}

fn run_window_thread(
    config: RendererConfig,
    ready_tx: Sender<Result<EventLoopProxy<WindowCommand>, anyhow::Error>>,
    signal_tx: Sender<WindowSignal>,
) -> Result<()> {
    let mut builder = EventLoopBuilder::<WindowCommand>::with_user_event();
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        use winit::platform::wayland::EventLoopBuilderExtWayland;
        EventLoopBuilderExtWayland::with_any_thread(&mut builder, true);
    }

    #[cfg(any(
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    ))]
    {
        use winit::platform::x11::EventLoopBuilderExtX11;
        EventLoopBuilderExtX11::with_any_thread(&mut builder, true);
    }
    let event_loop = builder
        .build()
        .map_err(|err| anyhow!("failed to create event loop: {err}"))?;
    let proxy = event_loop.create_proxy();

    let window_size = PhysicalSize::new(config.surface_size.0, config.surface_size.1);
    let mut builder = WindowBuilder::new()
        .with_title("Lambda Shader Preview")
        .with_inner_size(window_size);
    if !config.show_window {
        builder = builder.with_visible(false);
    }
    let window = builder
        .build(&event_loop)
        .map_err(|err| anyhow!("failed to create preview window: {err}"))?;
    let window = Arc::new(window);

    let mut state = match WindowState::new(window.clone(), &config) {
        Ok(state) => state,
        Err(err) => {
            let wrapped = anyhow!("failed to initialise window renderer: {err}");
            let message = wrapped.to_string();
            let _ = ready_tx.send(Err(anyhow!(message)));
            return Err(wrapped);
        }
    };

    let profile = state.adapter_profile().clone();
    let mut effective_policy = config.policy.clone();
    let mut software_cap = None;
    if profile.is_software() {
        let user_override = config.target_fps.is_some()
            || !matches!(config.policy, RenderPolicy::Animate { .. })
            || matches!(
                config.policy,
                RenderPolicy::Animate {
                    target_fps: Some(_),
                    ..
                }
            );
        if !user_override {
            software_cap = Some(SOFTWARE_FPS_CAP);
            if let RenderPolicy::Animate { target_fps, .. } = &mut effective_policy {
                *target_fps = Some(SOFTWARE_FPS_CAP);
            }
        }
    }

    let mut current_policy = effective_policy.clone();
    let mut policy_driver = RenderPolicyDriver::new(effective_policy)?;
    if let Some(cap) = software_cap {
        tracing::warn!(
            adapter = %profile.name,
            backend = ?profile.backend,
            cap,
            "software rasterizer detected; capping window preview to {} FPS (override with --fps)",
            cap
        );
    }
    if policy_driver.ready_for_frame(Instant::now()) {
        state.window().request_redraw();
    }

    let _ = ready_tx.send(Ok(proxy.clone()));

    let mut result = Ok(());
    let run_result = event_loop.run(move |event, elwt| {
        match event {
            Event::UserEvent(command) => match command {
                WindowCommand::Swap { request } => {
                    let SwapRequest {
                        shader_source,
                        channel_bindings,
                        crossfade,
                        antialiasing,
                        surface_alpha,
                        color_space,
                        warmup,
                        policy,
                        ..
                    } = request;

                    if let Err(err) = state.set_shader(
                        shader_source.as_path(),
                        &channel_bindings,
                        antialiasing,
                        surface_alpha,
                        color_space,
                        crossfade,
                        warmup,
                    ) {
                        error!("failed to swap window shader: {err:?}");
                    } else {
                        let updated_driver = match RenderPolicyDriver::new(policy.clone()) {
                            Ok(driver) => driver,
                            Err(err) => {
                                error!("failed to update render policy: {err:?}");
                                policy_driver.reset();
                                return;
                            }
                        };
                        policy_driver = updated_driver;
                        current_policy = policy.clone();
                        if policy_driver.ready_for_frame(Instant::now()) {
                            state.window().request_redraw();
                        }
                    }
                }
                WindowCommand::Shutdown => {
                    elwt.exit();
                }
            },
            Event::WindowEvent { window_id, event } if window_id == state.window().id() => {
                match event {
                    WindowEvent::CloseRequested | WindowEvent::Destroyed => {
                        elwt.exit();
                    }
                    WindowEvent::KeyboardInput { event, .. } => {
                        let keyboard_changed = state.keyboard.handle_event(&event);
                        if keyboard_changed {
                            state.sync_keyboard(false);
                        }
                        if event.state == ElementState::Pressed && !event.repeat {
                            let is_space = matches!(event.logical_key, Key::Named(NamedKey::Space))
                                || matches!(event.logical_key, Key::Character(ref value) if value.as_str() == " ");
                            if is_space {
                                let _ = signal_tx.send(WindowSignal::AdvancePlaylist);
                            }
                        }
                    }
                    WindowEvent::CursorMoved { position, .. } => {
                        state.handle_cursor_moved(position);
                    }
                    WindowEvent::MouseInput {
                        state: button_state,
                        button,
                        ..
                    } => {
                        if button == winit::event::MouseButton::Left {
                            state.handle_mouse_button(button_state);
                        }
                    }
                    WindowEvent::Resized(new_size) => {
                        let target_size = if !config.show_window
                            && matches!(current_policy, RenderPolicy::Export { .. })
                        {
                            PhysicalSize::new(config.surface_size.0, config.surface_size.1)
                        } else {
                            new_size
                        };
                        state.resize(target_size);
                    }
                    WindowEvent::ScaleFactorChanged {
                        mut inner_size_writer,
                        ..
                    } => {
                        let _ = inner_size_writer.request_inner_size(state.size());
                    }
                    WindowEvent::RedrawRequested => {
                        match state.render_frame(policy_driver.sample()) {
                            Ok(RenderFrameStatus::Presented) => {
                                policy_driver.mark_rendered();
                            }
                            Ok(RenderFrameStatus::Captured(path)) => {
                                policy_driver.mark_rendered();
                                tracing::info!("still frame captured at {}", path.display());
                                if config.exit_on_export {
                                    elwt.exit();
                                }
                            }
                            Err(err) => {
                                if let Some(surface_err) = err.as_surface_error() {
                                    match surface_err {
                                        wgpu::SurfaceError::Lost
                                        | wgpu::SurfaceError::Outdated => {
                                            state.resize(state.size());
                                        }
                                        wgpu::SurfaceError::OutOfMemory => {
                                            eprintln!(
                                                "surface out of memory; exiting preview"
                                            );
                                            elwt.exit();
                                        }
                                        wgpu::SurfaceError::Timeout => {
                                            eprintln!("surface timeout; retrying next frame");
                                        }
                                        other => {
                                            eprintln!(
                                                "surface error: {other:?}; retrying next frame"
                                            );
                                        }
                                    }
                                } else {
                                    tracing::error!(error = %err, "failed to export still frame");
                                    elwt.exit();
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            Event::AboutToWait => {
                let now = Instant::now();
                if policy_driver.ready_for_frame(now) {
                    tracing::trace!("scheduler: issuing redraw now");
                    state.window().request_redraw();
                    elwt.set_control_flow(ControlFlow::Wait);
                } else if let Some(deadline) = policy_driver.next_deadline() {
                    let ms = deadline.saturating_duration_since(now).as_millis();
                    tracing::trace!(deadline_ms = ms, "scheduler: waiting until next frame");
                    elwt.set_control_flow(ControlFlow::WaitUntil(deadline));
                } else {
                    tracing::trace!("scheduler: idle (no redraw requested)");
                    elwt.set_control_flow(ControlFlow::Wait);
                }
            }
            _ => {}
        }
    });

    if let Err(err) = run_result {
        result = Err(anyhow!("window event loop error: {err}"));
    }

    result
}

#[derive(Default)]
struct MouseState {
    position: Option<PhysicalPosition<f64>>,
    pressed_anchor: Option<PhysicalPosition<f64>>,
    is_pressed: bool,
}

impl MouseState {
    fn handle_cursor_moved(&mut self, position: PhysicalPosition<f64>) {
        self.position = Some(position);
        if self.is_pressed {
            self.pressed_anchor.get_or_insert(position);
        }
    }

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

const KEYBOARD_WIDTH: usize = 256;
const KEYBOARD_HEIGHT: usize = 3;
const KEYBOARD_CHANNELS: usize = 4;
const KEYBOARD_ROW_STATE: usize = 0;
const KEYBOARD_ROW_PULSE: usize = 1;
const KEYBOARD_ROW_TOGGLE: usize = 2;

struct KeyboardState {
    pressed: [u8; KEYBOARD_WIDTH],
    toggled: [u8; KEYBOARD_WIDTH],
    pulse_pending: [bool; KEYBOARD_WIDTH],
    data: Vec<u8>,
    dirty: bool,
}

impl Default for KeyboardState {
    fn default() -> Self {
        Self {
            pressed: [0; KEYBOARD_WIDTH],
            toggled: [0; KEYBOARD_WIDTH],
            pulse_pending: [false; KEYBOARD_WIDTH],
            data: vec![0u8; KEYBOARD_WIDTH * KEYBOARD_HEIGHT * KEYBOARD_CHANNELS],
            dirty: false,
        }
    }
}

impl KeyboardState {
    fn handle_event(&mut self, event: &KeyEvent) -> bool {
        let Some(code) = ascii_from_key_event(event) else {
            return false;
        };
        let index = code as usize;

        let changed = match event.state {
            ElementState::Pressed => {
                if event.repeat || self.pressed[index] == 255 {
                    return false;
                }

                self.pressed[index] = 255;
                self.write_cell(KEYBOARD_ROW_STATE, index, 255);
                self.write_cell(KEYBOARD_ROW_PULSE, index, 255);
                self.pulse_pending[index] = true;

                let new_value = if self.toggled[index] == 0 { 255 } else { 0 };
                self.toggled[index] = new_value;
                self.write_cell(KEYBOARD_ROW_TOGGLE, index, new_value);
                true
            }
            ElementState::Released => {
                if self.pressed[index] == 0 {
                    return false;
                }

                self.pressed[index] = 0;
                self.write_cell(KEYBOARD_ROW_STATE, index, 0);
                true
            }
        };

        if changed {
            self.dirty = true;
        }
        changed
    }

    fn snapshot(&self) -> Vec<u8> {
        self.data.clone()
    }

    fn take_dirty_snapshot(&mut self) -> Option<Vec<u8>> {
        if self.dirty {
            self.dirty = false;
            Some(self.data.clone())
        } else {
            None
        }
    }

    fn clear_dirty(&mut self) {
        self.dirty = false;
        let had_pulses = self.pulse_pending.iter().any(|&pending| pending);
        if had_pulses {
            for index in 0..KEYBOARD_WIDTH {
                if self.pulse_pending[index] {
                    self.write_cell(KEYBOARD_ROW_PULSE, index, 0);
                }
            }
        }
        self.pulse_pending.fill(false);
    }

    fn take_pulse_reset_snapshot(&mut self) -> Option<Vec<u8>> {
        let mut any = false;
        for index in 0..KEYBOARD_WIDTH {
            if self.pulse_pending[index] {
                self.pulse_pending[index] = false;
                self.write_cell(KEYBOARD_ROW_PULSE, index, 0);
                any = true;
            }
        }

        if any {
            Some(self.data.clone())
        } else {
            None
        }
    }

    fn write_cell(&mut self, row: usize, column: usize, value: u8) {
        debug_assert!(row < KEYBOARD_HEIGHT, "keyboard row out of range");
        debug_assert!(column < KEYBOARD_WIDTH, "keyboard column out of range");
        let stride = KEYBOARD_WIDTH * KEYBOARD_CHANNELS;
        let offset = row * stride + column * KEYBOARD_CHANNELS;
        self.data[offset..offset + KEYBOARD_CHANNELS].fill(value);
    }
}

fn ascii_from_key_event(event: &KeyEvent) -> Option<u8> {
    match &event.logical_key {
        Key::Character(value) if !value.is_empty() => {
            let mut chars = value.chars();
            let ch = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            if ch.is_ascii() {
                Some(ch as u8)
            } else {
                None
            }
        }
        Key::Named(NamedKey::Space) => Some(b' '),
        Key::Named(NamedKey::Enter) => Some(b'\n'),
        Key::Named(NamedKey::Tab) => Some(b'\t'),
        Key::Named(NamedKey::Backspace) => Some(8),
        Key::Named(NamedKey::Escape) => Some(27),
        _ => None,
    }
}
