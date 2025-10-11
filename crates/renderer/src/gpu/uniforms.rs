use std::time::Instant;

use bytemuck::{Pod, Zeroable};
use chrono::{Datelike, Local, Timelike};
use winit::dpi::PhysicalSize;

use crate::runtime::TimeSample;
use crate::types::CHANNEL_COUNT;

#[repr(C, align(8))]
#[derive(Clone, Copy)]
pub(crate) struct Std140Vec2 {
    value: [f32; 2],
}

unsafe impl Zeroable for Std140Vec2 {}
unsafe impl Pod for Std140Vec2 {}

#[repr(C, align(16))]
#[derive(Clone, Copy)]
pub(crate) struct ShadertoyUniforms {
    pub i_resolution: [f32; 4],
    pub i_time: f32,
    pub i_time_delta: f32,
    pub i_frame: i32,
    pub i_padding0: f32,
    pub i_mouse: [f32; 4],
    pub i_date: [f32; 4],
    pub i_sample_rate: f32,
    pub i_fade: f32,
    pub i_padding1: Std140Vec2,
    pub i_channel_time: [[f32; 4]; CHANNEL_COUNT],
    pub i_channel_resolution: [[f32; 4]; CHANNEL_COUNT],
    pub i_surface: [f32; 4],
    pub i_fill: [f32; 4],
    pub i_fill_wrap: [f32; 4],
}

unsafe impl Zeroable for ShadertoyUniforms {}
unsafe impl Pod for ShadertoyUniforms {}

impl ShadertoyUniforms {
    pub fn new(width: u32, height: u32) -> Self {
        let mut uniforms = Self {
            i_resolution: [width as f32, height as f32, 0.0, 0.0],
            i_time: 0.0,
            i_time_delta: 0.0,
            i_frame: 0,
            i_padding0: 0.0,
            i_mouse: [0.0; 4],
            i_date: [0.0; 4],
            i_sample_rate: 44100.0,
            i_fade: 1.0,
            i_padding1: Std140Vec2 { value: [0.0, 0.0] },
            i_channel_time: [[0.0; 4]; CHANNEL_COUNT],
            i_channel_resolution: [[0.0; 4]; CHANNEL_COUNT],
            i_surface: [width as f32, height as f32, width as f32, height as f32],
            i_fill: [1.0, 1.0, 0.0, 0.0],
            i_fill_wrap: [0.0, 0.0, 0.0, 0.0],
        };
        uniforms.refresh_date();
        uniforms
    }

    pub fn set_resolution(&mut self, width: f32, height: f32) {
        self.i_resolution[0] = width;
        self.i_resolution[1] = height;
    }

    pub fn set_surface(
        &mut self,
        surface_width: f32,
        surface_height: f32,
        logical_width: f32,
        logical_height: f32,
    ) {
        self.i_surface[0] = surface_width;
        self.i_surface[1] = surface_height;
        self.i_surface[2] = logical_width;
        self.i_surface[3] = logical_height;
    }

    pub fn set_fill(&mut self, scale_x: f32, scale_y: f32, offset_x: f32, offset_y: f32) {
        self.i_fill[0] = scale_x;
        self.i_fill[1] = scale_y;
        self.i_fill[2] = offset_x;
        self.i_fill[3] = offset_y;
    }

    pub fn set_fill_wrap(&mut self, wrap_x: f32, wrap_y: f32) {
        self.i_fill_wrap[0] = wrap_x;
        self.i_fill_wrap[1] = wrap_y;
    }

    pub fn set_fade(&mut self, fade: f32) {
        self.i_fade = fade;
    }

    pub fn set_channel_resolution(&mut self, index: usize, resolution: [f32; 4]) {
        if let Some(slot) = self.i_channel_resolution.get_mut(index) {
            *slot = resolution;
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_time(
        &mut self,
        start_time: &mut Instant,
        last_frame_time: &mut Instant,
        frame_count: &mut u32,
        last_override_sample: &mut Option<TimeSample>,
        now: Instant,
        override_sample: Option<TimeSample>,
        mouse: [f32; 4],
    ) {
        let (seconds, delta_seconds, frame_index, next_frame_count) =
            if let Some(sample) = override_sample {
                let previous = last_override_sample.replace(sample);
                let delta = previous
                    .map(|prev| (sample.seconds - prev.seconds).max(0.0))
                    .unwrap_or(0.0);

                *start_time = now;
                *last_frame_time = now;
                let frame_value = sample.frame_index.min(i32::MAX as u64) as i32;
                let next_count = sample
                    .frame_index
                    .saturating_add(1)
                    .min(u64::from(u32::MAX)) as u32;
                (sample.seconds, delta, frame_value, next_count)
            } else {
                *last_override_sample = None;
                if *frame_count == 0 {
                    *start_time = now;
                    *last_frame_time = now;
                }
                let elapsed = now.duration_since(*start_time);
                let delta = now.duration_since(*last_frame_time);
                *last_frame_time = now;
                let frame_index = *frame_count as i32;
                let next_count = frame_count.saturating_add(1);
                (
                    elapsed.as_secs_f32(),
                    delta.as_secs_f32(),
                    frame_index,
                    next_count,
                )
            };

        self.i_time = seconds;
        self.i_time_delta = delta_seconds;
        self.i_frame = frame_index;
        *frame_count = next_frame_count;
        for channel in &mut self.i_channel_time {
            channel[0] = self.i_time;
        }
        self.i_mouse = mouse;
        self.i_resolution[3] = self.i_time;
        self.refresh_date();
    }

    fn refresh_date(&mut self) {
        let local_now = Local::now();
        let seconds_since_midnight = local_now.num_seconds_from_midnight() as f32
            + local_now.nanosecond() as f32 / 1_000_000_000.0;
        self.i_date = [
            local_now.year() as f32,
            local_now.month() as f32,
            local_now.day() as f32,
            seconds_since_midnight,
        ];
    }
}

pub(crate) fn logical_dimensions(
    fill_scale: f32,
    fill_method: crate::runtime::FillMethod,
    surface: PhysicalSize<u32>,
) -> (f32, f32) {
    let surface_w = surface.width.max(1) as f32;
    let surface_h = surface.height.max(1) as f32;
    match fill_method {
        crate::runtime::FillMethod::Stretch | crate::runtime::FillMethod::Tile { .. } => {
            (surface_w * fill_scale, surface_h * fill_scale)
        }
        crate::runtime::FillMethod::Center {
            content_width,
            content_height,
        } => (
            (content_width as f32).max(1.0) * fill_scale,
            (content_height as f32).max(1.0) * fill_scale,
        ),
    }
}

pub(crate) fn fill_parameters(
    fill_scale: f32,
    fill_method: crate::runtime::FillMethod,
    surface: PhysicalSize<u32>,
    logical: (f32, f32),
) -> (f32, f32, f32, f32, f32, f32) {
    let surface_w = surface.width.max(1) as f32;
    let surface_h = surface.height.max(1) as f32;
    let (logical_w, logical_h) = logical;
    let mut scale_x = if surface_w > 0.0 {
        logical_w / surface_w
    } else {
        fill_scale.max(0.0001)
    };
    let mut scale_y = if surface_h > 0.0 {
        logical_h / surface_h
    } else {
        fill_scale.max(0.0001)
    };
    let mut offset_x = 0.0_f32;
    let mut offset_y = 0.0_f32;
    let mut wrap_x = 0.0_f32;
    let mut wrap_y = 0.0_f32;

    match fill_method {
        crate::runtime::FillMethod::Stretch => {}
        crate::runtime::FillMethod::Center {
            content_width,
            content_height,
        } => {
            let content_w = (content_width as f32).max(1.0);
            let content_h = (content_height as f32).max(1.0);
            let content_physical_w = content_w.min(surface_w);
            let content_physical_h = content_h.min(surface_h);

            if content_physical_w > 0.0 {
                scale_x = (content_w * fill_scale) / content_physical_w;
            }
            if content_physical_h > 0.0 {
                scale_y = (content_h * fill_scale) / content_physical_h;
            }

            let left = (surface_w - content_physical_w) * 0.5;
            let bottom = (surface_h - content_physical_h) * 0.5;
            offset_x = -left * scale_x;
            offset_y = -bottom * scale_y;
        }
        crate::runtime::FillMethod::Tile { repeat_x, repeat_y } => {
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

    (scale_x, scale_y, offset_x, offset_y, wrap_x, wrap_y)
}
