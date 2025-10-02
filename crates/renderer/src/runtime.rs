use std::path::PathBuf;
use std::time::Instant;

use anyhow::{anyhow, Result};

use crate::types::{ColorSpaceMode, ShaderCompiler};

/// High-level behaviour requested by the caller.
///
/// The render policy decides whether frames should animate continuously,
/// be evaluated at a fixed timestamp, or be exported to disk.
#[derive(Debug, Clone, PartialEq)]
pub enum RenderPolicy {
    /// Run the render loop continuously, optionally clamping the frame rate or
    /// enabling adaptive throttling when occluded.
    Animate {
        /// Optional requested frames-per-second cap.
        target_fps: Option<f32>,
        /// When true the engine may drop FPS when surfaces are hidden.
        adaptive: bool,
    },
    /// Render a single still frame at an optional timestamp.
    Still {
        /// Specific timestamp to evaluate the shader at (seconds).
        time: Option<f32>,
        /// Optional deterministic seed for shaders that rely on randomness.
        random_seed: Option<u64>,
    },
    /// Render a frame and write the result to disk.
    Export {
        /// Specific timestamp to evaluate the shader at (seconds).
        time: Option<f32>,
        /// Destination path for the exported file.
        path: PathBuf,
        /// Output format the user requested.
        format: ExportFormat,
    },
}

impl Default for RenderPolicy {
    fn default() -> Self {
        Self::Animate {
            target_fps: None,
            adaptive: false,
        }
    }
}

/// File formats supported by the still/export pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Png,
    Exr,
}

/// Spatial mapping from shader coordinates onto the wallpaper surface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FillMethod {
    /// Stretch shader output to fill the surface.
    Stretch,
    /// Center the shader at a fixed content resolution, letterboxing otherwise.
    Center {
        /// Native shader resolution before letterboxing.
        content_width: u32,
        content_height: u32,
    },
    /// Tile the shader output across the surface.
    Tile {
        /// Horizontal repeat count; 1.0 means exactly once per surface width.
        repeat_x: f32,
        /// Vertical repeat count; 1.0 means exactly once per surface height.
        repeat_y: f32,
    },
}

impl Default for FillMethod {
    fn default() -> Self {
        Self::Stretch
    }
}

/// Options that fine-tune renderer quality and run-time behaviour.
#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeOptions {
    /// Supersampling factor applied before presenting to the surface.
    pub render_scale: f32,
    /// How shader coordinates map to the wallpaper surface.
    pub fill_method: FillMethod,
    /// Maximum FPS to allow when the wallpaper is hidden or throttled.
    pub max_fps_occluded: Option<f32>,
    /// Desired output colour handling.
    pub color_space: ColorSpaceMode,
    /// Shader compilation backend preference.
    pub shader_compiler: ShaderCompiler,
}

impl RuntimeOptions {
    /// Builds a runtime options struct with sensible defaults.
    pub fn new() -> Self {
        Self::default()
    }
}

impl Default for RuntimeOptions {
    fn default() -> Self {
        Self {
            render_scale: 1.0,
            fill_method: FillMethod::default(),
            max_fps_occluded: None,
            color_space: ColorSpaceMode::default(),
            shader_compiler: ShaderCompiler::default(),
        }
    }
}

/// Snapshot of the time state supplied to the shader uniforms.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimeSample {
    /// Elapsed wall-clock or simulated time in seconds.
    pub seconds: f32,
    /// Monotonic frame counter for the running session.
    pub frame_index: u64,
}

impl TimeSample {
    /// Creates a new time sample.
    pub fn new(seconds: f32, frame_index: u64) -> Self {
        Self {
            seconds,
            frame_index,
        }
    }
}

/// Abstraction over where time values originate from.
pub trait TimeSource: Send {
    /// Resets the source to its initial state.
    fn reset(&mut self);
    /// Produces a time sample for the next frame.
    fn sample(&mut self) -> TimeSample;
}

/// Time source backed by the system monotonic clock.
#[derive(Debug, Clone, Copy)]
pub struct SystemTimeSource {
    origin: Instant,
    frame: u64,
}

impl SystemTimeSource {
    /// Creates a system time source initialised to `Instant::now()`.
    pub fn new() -> Self {
        Self::default()
    }
}

impl Default for SystemTimeSource {
    fn default() -> Self {
        Self {
            origin: Instant::now(),
            frame: 0,
        }
    }
}

impl TimeSource for SystemTimeSource {
    fn reset(&mut self) {
        self.origin = Instant::now();
        self.frame = 0;
    }

    fn sample(&mut self) -> TimeSample {
        let elapsed = self.origin.elapsed();
        let sample = TimeSample::new(elapsed.as_secs_f32(), self.frame);
        self.frame = self.frame.saturating_add(1);
        sample
    }
}

/// Time source that always reports a fixed timestamp.
#[derive(Debug, Clone, Copy)]
pub struct FixedTimeSource {
    time: f32,
}

impl FixedTimeSource {
    /// Constructs a fixed time source that always returns the provided time.
    pub fn new(time: f32) -> Self {
        Self { time }
    }

    /// Accesses the fixed timestamp without advancing the frame counter.
    pub fn time(&self) -> f32 {
        self.time
    }
}

impl TimeSource for FixedTimeSource {
    fn reset(&mut self) {}

    fn sample(&mut self) -> TimeSample {
        TimeSample::new(self.time, 0)
    }
}

/// Convenient alias for owning time sources behind trait objects.
pub type BoxedTimeSource = Box<dyn TimeSource + Send>;

/// Builds a time source suited to the requested render policy.
pub fn time_source_for_policy(policy: &RenderPolicy) -> Result<BoxedTimeSource> {
    match policy {
        RenderPolicy::Animate { .. } => Ok(Box::new(SystemTimeSource::new())),
        RenderPolicy::Still { time, .. } => Ok(Box::new(FixedTimeSource::new(time.unwrap_or(0.0)))),
        RenderPolicy::Export { .. } => Err(anyhow!("export policy is not implemented")),
    }
}
