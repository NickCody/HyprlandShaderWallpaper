//! Renderer crate for Lambda Shade (Lambda Shade).
//!
//! The module glues the Wayland preview window, `wgpu` rendering pipeline, and
//! ShaderToy-compatible shader wrapping together. The overall flow is:
//!
//! ```text
//!   CLI / lambdash
//!          │ RendererConfig
//!          ▼
//!   Renderer::run ──▶ WindowState ──▶ winit event loop ──▶ render_frame()
//!          ▲                                      │
//!          │                                      └─▶ update_uniforms() ─▶ GPU UBO
//! ```
//!
//! `WindowState` owns all GPU resources (surface, device, pipeline, uniforms),
//! while `Renderer` is the thin entry point that chooses between wallpaper mode
//! or the interactive preview window. The fragment shaders downloaded from
//! ShaderToy are wrapped at runtime so they can be compiled as Vulkan GLSL and
//! fed the expected uniforms and texture bindings.

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
    Antialiasing, ChannelBindings, ChannelSource, ChannelTextureKind, ColorSpaceMode, RenderMode,
    RendererConfig, ShaderCompiler, SurfaceAlpha, CUBEMAP_FACE_STEMS,
};
pub use wallpaper::{
    OutputId, SurfaceId, SurfaceInfo, SurfaceSelector, SwapRequest, WallpaperRuntime,
};
pub use window::WindowRuntime;

use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use winit::dpi::PhysicalSize;
use winit::event::{Event, MouseButton, WindowEvent};
use winit::event_loop::{ControlFlow, EventLoop};
use winit::window::WindowBuilder;

use window::{RenderPolicyDriver, WindowState};

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
            RenderMode::Windowed => self.run_window_preview(),
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
        let window = WindowBuilder::new()
            .with_title("Lambda Shade Preview")
            .with_inner_size(window_size)
            .build(&event_loop)
            .context("failed to create preview window")?;
        let window = Arc::new(window);

        let mut state = WindowState::new(window.clone(), &self.config)?;
        let mut policy_driver = RenderPolicyDriver::new(self.config.policy.clone())?;
        if policy_driver.should_request_redraw() {
            state.window().request_redraw();
        }

        event_loop
            .run(move |event, elwt| {
                elwt.set_control_flow(ControlFlow::Wait);

                match event {
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
                                state.resize(new_size);
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
                                    Ok(()) => {
                                        policy_driver.mark_rendered();
                                    }
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
                        if policy_driver.should_request_redraw() {
                            state.window().request_redraw();
                        }
                    }
                    _ => {}
                }
            })
            .map_err(|err| anyhow!("event loop error: {err}"))
    }
}
