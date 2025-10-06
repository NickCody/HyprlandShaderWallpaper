//! Renderer crate for WallShader (Wayland wallpaper engine + preview window).
//!
//! This crate ties together the wgpu rendering backend, a ShaderToy-compatible
//! shader wrapper, and two front-ends: a Wayland wallpaper path and a winit-based
//! preview window. It exposes a small `Renderer` entry point and re-exports types
//! commonly needed by the daemon.
//!
//! Big picture
//!
//! ```text
//! wallshader (daemon/CLI)
//!        │  builds
//!        ▼
//!  renderer::Renderer ────────────────┐
//!        │                             │
//!   RenderMode::Windowed               │ RenderMode::Wallpaper
//!        │                             │
//!        ▼                             ▼
//!  window::WindowRuntime         wallpaper::WallpaperRuntime
//!        │                             │
//!        └──▶ gpu::GpuState ◀──────────┘   (device/surface, uniforms, pipelines)
//!                         ▲
//!                         └── compile::{vertex,fragment} (wrap + compile GLSL)
//! ```
//!
//! Roles
//!
//! - `Renderer` — thin coordinator that selects preview vs wallpaper and delegates run.
//! - `window`   — preview event loop, input handling, still/export capture.
//! - `wallpaper`— Wayland layer surfaces per output, frame pacing, multi-surface swaps.
//! - `gpu`      — wgpu orchestration: device/surface, pipelines, uniforms, channels.
//! - `compile`  — wraps ShaderToy fragment code and compiles GLSL.
//! - `runtime`  — render policy, time sources, fill method, frame scheduling helpers.
//!
//! The fragment shaders are wrapped at runtime (uniform block + macros) to align with
//! ShaderToy semantics (`iTime`, `iMouse`, `iChannel*`), then compiled to a
//! `wgpu::ShaderModule` (via naga or shaderc) before drawing.

mod compile;
mod gpu;
mod runtime;
mod types;
mod wallpaper;
mod window;

pub use runtime::{
    time_source_for_policy, BoxedTimeSource, ExportFormat, FillMethod, FixedTimeSource,
    RenderPolicy, RuntimeOptions, SystemTimeSource, TimeSample, TimeSource,
};
pub use types::{
    AdapterProfile, Antialiasing, ChannelBindings, ChannelSource, ChannelTextureKind,
    ColorSpaceMode, GpuMemoryMode, GpuPowerPreference, RenderMode, RendererConfig, ShaderCompiler,
    SurfaceAlpha, CUBEMAP_FACE_STEMS,
};
pub use wallpaper::{
    OutputId, SurfaceId, SurfaceInfo, SurfaceSelector, SwapRequest, WallpaperRuntime,
};
pub use window::WindowRuntime;

use std::sync::Arc;
use std::time::Instant;

use anyhow::{anyhow, Context, Result};
use winit::dpi::PhysicalSize;
use winit::event::{Event, MouseButton, WindowEvent};
use winit::event_loop::{ControlFlow, EventLoop};
use winit::window::WindowBuilder;

use window::{RenderFrameStatus, RenderPolicyDriver, WindowState};

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
    pub fn run(&mut self) -> Result<()> {
        match self.config.mode {
            RenderMode::Wallpaper => self.run_wallpaper(),
            RenderMode::Windowed => {
                if matches!(self.config.policy, RenderPolicy::Export { .. })
                    && self.config.exit_on_export
                {
                    self.run_export_once()
                } else {
                    self.run_window_preview()
                }
            }
        }
    }

    /// Drives the Wayland wallpaper path, rendering into a background layer surface.
    fn run_wallpaper(&self) -> Result<()> {
        wallpaper::run(&self.config)
    }

    /// Opens the preview window and drives the `winit` event loop.
    fn run_window_preview(&self) -> Result<()> {
        let event_loop = EventLoop::new().context("failed to initialize event loop")?;
        let window_size = PhysicalSize::new(self.config.surface_size.0, self.config.surface_size.1);
        let mut builder = WindowBuilder::new()
            .with_title("WallShader Preview")
            .with_inner_size(window_size);
        if !self.config.show_window {
            builder = builder.with_visible(false);
        }
        let window = builder
            .build(&event_loop)
            .context("failed to create preview window")?;
        let window = Arc::new(window);

        let mut state = WindowState::new(window.clone(), &self.config)?;
        let mut policy_driver = RenderPolicyDriver::new(self.config.policy.clone())?;
        if policy_driver.ready_for_frame(Instant::now()) {
            state.window().request_redraw();
        }

        event_loop
            .run(move |event, elwt| match event {
                Event::WindowEvent { window_id, event } if window_id == state.window().id() => {
                    match event {
                        WindowEvent::CloseRequested | WindowEvent::Destroyed => {
                            elwt.exit();
                        }
                        WindowEvent::CursorMoved { position, .. } => {
                            state.handle_cursor_moved(position);
                        }
                        WindowEvent::MouseInput {
                            state: button_state,
                            button,
                            ..
                        } => {
                            if button == MouseButton::Left {
                                state.handle_mouse_button(button_state);
                            }
                        }
                        WindowEvent::Resized(new_size) => {
                            let target_size = if !self.config.show_window
                                && matches!(self.config.policy, RenderPolicy::Export { .. })
                            {
                                PhysicalSize::new(
                                    self.config.surface_size.0,
                                    self.config.surface_size.1,
                                )
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
                            let sample = policy_driver.sample();
                            let render_result = state.render_frame(sample);
                            match render_result {
                                Ok(RenderFrameStatus::Presented) => {
                                    policy_driver.mark_rendered();
                                }
                                Ok(RenderFrameStatus::Captured(path)) => {
                                    policy_driver.mark_rendered();
                                    tracing::info!("still frame captured at {}", path.display());
                                    if self.config.exit_on_export {
                                        elwt.exit();
                                    }
                                }
                                Err(err) => {
                                    if let Some(surface_err) = err.as_surface_error() {
                                        match surface_err {
                                            wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated => {
                                                state.resize(state.size());
                                            }
                                            wgpu::SurfaceError::OutOfMemory => {
                                                eprintln!(
                                                    "surface out of memory; exiting preview"
                                                );
                                                elwt.exit();
                                            }
                                            wgpu::SurfaceError::Timeout => {
                                                eprintln!(
                                                    "surface timeout; retrying next frame"
                                                );
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
                        state.window().request_redraw();
                        elwt.set_control_flow(ControlFlow::Wait);
                    } else if let Some(deadline) = policy_driver.next_deadline() {
                        elwt.set_control_flow(ControlFlow::WaitUntil(deadline));
                    } else {
                        elwt.set_control_flow(ControlFlow::Wait);
                    }
                }
                _ => {}
            })
            .map_err(|err| anyhow!("event loop error: {err}"))
    }

    fn run_export_once(&self) -> Result<()> {
        let event_loop = EventLoop::new().context("failed to initialize event loop")?;
        let window_size = PhysicalSize::new(self.config.surface_size.0, self.config.surface_size.1);
        let builder = WindowBuilder::new()
            .with_title("WallShader Export")
            .with_inner_size(window_size)
            .with_visible(self.config.show_window);
        let window = builder
            .build(&event_loop)
            .context("failed to create export window")?;
        let window = Arc::new(window);

        let mut state = WindowState::new(window.clone(), &self.config)?;
        let mut policy_driver = RenderPolicyDriver::new(self.config.policy.clone())?;
        let sample = policy_driver.sample();
        match state.render_frame(sample)? {
            RenderFrameStatus::Captured(_) => {
                policy_driver.mark_rendered();
                Ok(())
            }
            RenderFrameStatus::Presented => Err(anyhow!(
                "export policy expected a captured frame but received a presentation"
            )),
        }
    }
}
