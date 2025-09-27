use std::sync::Arc;

use anyhow::Result;
use wgpu::SurfaceError;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::ElementState;
use winit::window::Window;

use crate::gpu::GpuState;
use crate::types::RendererConfig;

/// Aggregates GPU state for the windowed preview path.
pub(crate) struct WindowState {
    window: Arc<Window>,
    gpu: GpuState,
    mouse: MouseState,
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
        )?;

        Ok(Self {
            window,
            gpu,
            mouse: MouseState::default(),
        })
    }

    pub(crate) fn window(&self) -> &Window {
        self.window.as_ref()
    }

    pub(crate) fn size(&self) -> PhysicalSize<u32> {
        self.gpu.size()
    }

    pub(crate) fn resize(&mut self, new_size: PhysicalSize<u32>) {
        self.gpu.resize(new_size);
    }

    pub(crate) fn render_frame(&mut self) -> Result<(), SurfaceError> {
        let mouse_uniform = self.mouse.as_uniform(self.size().height.max(1) as f32);
        self.gpu.render(mouse_uniform)
    }

    pub(crate) fn handle_cursor_moved(&mut self, position: PhysicalPosition<f64>) {
        self.mouse.handle_cursor_moved(position);
    }

    pub(crate) fn handle_mouse_button(&mut self, state: ElementState) {
        self.mouse.handle_button(state);
    }
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
