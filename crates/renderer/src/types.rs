//! Renderer data model and configuration surface for wax11 shader.
//!
//! This module defines the types that describe what and how we render:
//! configuration passed in from the `wax11` daemon/CLI, ShaderToy-style
//! channel bindings, color/alpha preferences, GPU usage hints, and feature
//! toggles like anti-aliasing or fill method. Other renderer modules consume
//! these types to initialise GPU state and drive presentation.
//!
//! At a glance
//!
//! ```text
//! wax11 (CLI/daemon)
//!        │ builds
//!        ▼
//!  RendererConfig ──────────────┐
//!        │                      │
//!        │                      ├──▶ gpu::GpuState    (creates device/surface, pipelines)
//!        │                      │
//!        │                      └──▶ window::WindowState / wallpaper::SurfaceState
//!                                   (event loop, scheduling, input → uniforms)
//!
//! ChannelBindings ──▶ gpu::ShaderPipeline (bind group layout, textures/cubemaps/keyboard)
//! ```
//!
//! Details
//!
//! - `RendererConfig` mirrors CLI flags and playlist/manifest choices; it is the only
//!   input needed to construct the renderer.
//! - `ChannelBindings` describes ShaderToy `iChannel0..3` resources and yields a
//!   `layout_signature()` that gpu uses to build the correct bind group layout.
//! - GPU friendliness knobs (`GpuPowerPreference`, `GpuMemoryMode`, `gpu_latency`) reduce
//!   contention with foreground apps (browsers, games) by preferring low power, balanced
//!   memory usage, and modest frame buffering.
//!
//! Types summary
//!
//! - `RendererConfig` — immutable run configuration; consumed by `window`/`wallpaper`.
//! - `ChannelBindings` + `ChannelSource` + `CHANNEL_COUNT` — ShaderToy inputs.
//! - `ChannelTextureKind` + `CUBEMAP_FACE_STEMS` — texture dimensionality and discovery.
//! - `RenderMode`, `SurfaceAlpha`, `Antialiasing`, `ShaderCompiler`, `ColorSpaceMode` —
//!   rendering and colour handling policies.
//! - `GpuPowerPreference`, `GpuMemoryMode` — adapter/device usage hints.
//!
use std::path::PathBuf;

use anyhow::Result;

use crate::runtime::{FillMethod, RenderPolicy};
use wgpu::{AdapterInfo, Backend, DeviceType, Limits};

/// Adapter capabilities and metadata reported by wgpu.
#[derive(Debug, Clone)]
pub struct AdapterProfile {
    pub name: String,
    pub backend: Backend,
    pub device_type: DeviceType,
    pub driver: String,
    pub driver_info: String,
    pub limits: Limits,
}

impl AdapterProfile {
    pub fn from_wgpu(info: &AdapterInfo, limits: &Limits) -> Self {
        Self {
            name: info.name.clone(),
            backend: info.backend,
            device_type: info.device_type,
            driver: info.driver.clone(),
            driver_info: info.driver_info.clone(),
            limits: limits.clone(),
        }
    }

    pub fn is_software(&self) -> bool {
        matches!(self.device_type, DeviceType::Cpu)
            || self.name.to_ascii_lowercase().contains("llvmpipe")
            || self.name.to_ascii_lowercase().contains("softpipe")
            || self.driver.to_ascii_lowercase().contains("llvmpipe")
            || self.driver.to_ascii_lowercase().contains("softpipe")
    }
}

/// ShaderToy exposes four optional input channels (`iChannel0-3`).
pub const CHANNEL_COUNT: usize = 4;

/// Describes how a ShaderToy channel should be populated.
#[derive(Clone, Debug)]
pub enum ChannelSource {
    Texture { path: PathBuf },
    Cubemap { directory: PathBuf },
    Keyboard,
}

/// Enumerates the texture dimensionality requirements for a channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelTextureKind {
    Texture2d,
    Cubemap,
}

/// Expected face stems for cubemap resources stored on disk.
pub const CUBEMAP_FACE_STEMS: [&str; 6] = ["posx", "negx", "posy", "negy", "posz", "negz"];

/// Collection of ShaderToy channel bindings prepared for the renderer.
#[derive(Clone, Debug)]
pub struct ChannelBindings {
    sources: [Option<ChannelSource>; CHANNEL_COUNT],
}

impl ChannelBindings {
    /// Creates an empty bindings table with all channels unassigned.
    pub fn new() -> Self {
        Self::default()
    }

    /// Associates a texture path with the given channel.
    pub fn set_texture(&mut self, channel: usize, path: PathBuf) -> Result<()> {
        if channel >= CHANNEL_COUNT {
            anyhow::bail!(
                "channel {} exceeds supported ShaderToy channel count ({})",
                channel,
                CHANNEL_COUNT
            );
        }
        self.sources[channel] = Some(ChannelSource::Texture { path });
        Ok(())
    }

    /// Associates a cubemap directory with the given channel.
    pub fn set_cubemap(&mut self, channel: usize, directory: PathBuf) -> Result<()> {
        if channel >= CHANNEL_COUNT {
            anyhow::bail!(
                "channel {} exceeds supported ShaderToy channel count ({})",
                channel,
                CHANNEL_COUNT
            );
        }
        self.sources[channel] = Some(ChannelSource::Cubemap { directory });
        Ok(())
    }

    /// Marks the given channel as a virtual keyboard texture.
    pub fn set_keyboard(&mut self, channel: usize) -> Result<()> {
        if channel >= CHANNEL_COUNT {
            anyhow::bail!(
                "channel {} exceeds supported ShaderToy channel count ({})",
                channel,
                CHANNEL_COUNT
            );
        }
        self.sources[channel] = Some(ChannelSource::Keyboard);
        Ok(())
    }

    /// Exposes the underlying channel slots for GPU resource creation.
    pub(crate) fn slots(&self) -> &[Option<ChannelSource>; CHANNEL_COUNT] {
        &self.sources
    }

    /// Returns the required texture dimensionality for each channel.
    pub fn layout_signature(&self) -> [ChannelTextureKind; CHANNEL_COUNT] {
        let mut kinds = [ChannelTextureKind::Texture2d; CHANNEL_COUNT];
        for (index, source) in self.sources.iter().enumerate() {
            if matches!(source, Some(ChannelSource::Cubemap { .. })) {
                kinds[index] = ChannelTextureKind::Cubemap;
            }
        }
        kinds
    }
}

impl Default for ChannelBindings {
    fn default() -> Self {
        Self {
            sources: std::array::from_fn(|_| None),
        }
    }
}

/// Shader compilation backend requested by the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShaderCompiler {
    /// Compile wrapped GLSL through shaderc into SPIR-V (preferred for ShaderToy shaders).
    Shaderc,
    /// Hand GLSL to naga's built-in frontend.
    NagaGlsl,
}

impl Default for ShaderCompiler {
    fn default() -> Self {
        if cfg!(feature = "shaderc") {
            ShaderCompiler::Shaderc
        } else {
            ShaderCompiler::NagaGlsl
        }
    }
}

impl std::fmt::Display for ShaderCompiler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShaderCompiler::Shaderc => f.write_str("shaderc"),
            ShaderCompiler::NagaGlsl => f.write_str("naga"),
        }
    }
}

/// Output color handling for the renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorSpaceMode {
    /// Choose a sensible default based on ShaderToy expectations (gamma-encoded swapchain).
    #[default]
    Auto,
    /// Treat shader outputs/textures as gamma-encoded; use non-sRGB surfaces.
    Gamma,
    /// Treat shader outputs as linear and use sRGB swapchains/textures for conversion.
    Linear,
}

/// How the renderer should present frames.
///
/// * `Wallpaper` will eventually stream frames directly into a Wayland layer
///   or xdg-shell surface owned by the compositor. This path is not implemented
///   yet, so we politely warn the caller instead of silently failing.
/// * `Windowed` spins up an interactive preview window driven by `winit` so we
///   can debug shaders on a desktop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderMode {
    Wallpaper,
    Windowed,
}

/// Declares how the compositor should treat the swapchain alpha channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceAlpha {
    /// Frames fully cover the wallpaper surface without transparency.
    Opaque,
    /// Frames may contain transparency and should be blended by the compositor.
    Transparent,
}

/// Envelope applied to crossfades between shaders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CrossfadeCurve {
    /// Linear progression from 0 → 1.
    Linear,
    /// Smoothstep easing (default) for gentle acceleration/deceleration.
    #[default]
    Smoothstep,
    /// Quadratic ease-in/ease-out.
    EaseInOut,
}

impl Default for SurfaceAlpha {
    fn default() -> Self {
        Self::Opaque
    }
}

/// Anti-aliasing policy for the render pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Antialiasing {
    /// Pick the highest sample count supported by the surface format.
    Auto,
    /// Disable MSAA and render directly into the swapchain.
    Off,
    /// Request a specific MSAA sample count (clamped to what the device supports).
    Samples(u32),
}

impl Default for Antialiasing {
    fn default() -> Self {
        Self::Auto
    }
}

/// GPU power preference for adapter selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuPowerPreference {
    /// Low power mode, friendly to other applications (default).
    Low,
    /// High performance mode, maximum GPU priority.
    High,
}

impl Default for GpuPowerPreference {
    fn default() -> Self {
        Self::Low
    }
}

/// GPU memory allocation mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuMemoryMode {
    /// Balanced memory usage, friendly to other applications (default).
    Balanced,
    /// Performance mode, maximum memory allocation priority.
    Performance,
}

impl Default for GpuMemoryMode {
    fn default() -> Self {
        Self::Balanced
    }
}

/// Immutable configuration passed to the renderer at start-up.
///
/// `RendererConfig` mirrors CLI flags and tells the renderer which shader file
/// to compile, how large the target surface should be, and which presentation
/// mode to use.
#[derive(Clone)]
pub struct RendererConfig {
    /// Window or surface size in physical pixels.
    pub surface_size: (u32, u32),
    /// Path to the ShaderToy-style fragment shader that should be rendered.
    pub shader_source: PathBuf,
    /// Presentation mode (wallpaper vs interactive window).
    pub mode: RenderMode,
    /// Optional resolution explicitly requested by the caller.
    pub requested_size: Option<(u32, u32)>,
    /// Optional FPS cap for wallpaper mode; None = render every callback.
    pub target_fps: Option<f32>,
    /// Optional ShaderToy channel bindings (textures, cubemaps, etc.).
    pub channel_bindings: ChannelBindings,
    /// Anti-aliasing mode requested by the caller.
    pub antialiasing: Antialiasing,
    /// Alpha behaviour of the surface from the manifest or CLI.
    pub surface_alpha: SurfaceAlpha,
    /// Shader compiler that should be used for wrapped GLSL.
    pub shader_compiler: ShaderCompiler,
    /// Desired color handling for swapchain/textures.
    pub color_space: ColorSpaceMode,
    /// Supersampling factor applied before presenting to the surface (1.0 = native).
    pub render_scale: f32,
    /// How shader coordinates map to the wallpaper surface.
    pub fill_method: FillMethod,
    /// Maximum FPS allowed while the surface is occluded (if adaptive throttling enabled).
    pub max_fps_occluded: Option<f32>,
    /// Whether the preview window should be visible when created.
    pub show_window: bool,
    /// Whether the renderer should exit automatically after completing an export capture.
    pub exit_on_export: bool,
    /// High-level render behaviour requested by the caller.
    pub policy: RenderPolicy,
    /// Shape to use when crossfading between shaders.
    pub crossfade_curve: CrossfadeCurve,
    /// GPU power preference for adapter selection.
    pub gpu_power: GpuPowerPreference,
    /// GPU memory allocation mode.
    pub gpu_memory: GpuMemoryMode,
    /// GPU frame latency (number of frames buffered).
    pub gpu_latency: u32,
}

impl Default for RendererConfig {
    /// Provides a 1080p windowed configuration with no shader selected.
    fn default() -> Self {
        Self {
            surface_size: (1920, 1080),
            shader_source: PathBuf::new(),
            mode: RenderMode::Wallpaper,
            requested_size: None,
            target_fps: None,
            channel_bindings: ChannelBindings::default(),
            antialiasing: Antialiasing::default(),
            surface_alpha: SurfaceAlpha::Opaque,
            shader_compiler: ShaderCompiler::default(),
            color_space: ColorSpaceMode::default(),
            render_scale: 1.0,
            fill_method: FillMethod::default(),
            max_fps_occluded: None,
            show_window: true,
            exit_on_export: true,
            policy: RenderPolicy::default(),
            crossfade_curve: CrossfadeCurve::default(),
            gpu_power: GpuPowerPreference::default(),
            gpu_memory: GpuMemoryMode::default(),
            gpu_latency: 2,
        }
    }
}
