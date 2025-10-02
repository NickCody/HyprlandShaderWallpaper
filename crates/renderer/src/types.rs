use std::path::PathBuf;

use anyhow::Result;

use crate::runtime::RenderPolicy;

/// ShaderToy exposes four optional input channels (`iChannel0-3`).
pub const CHANNEL_COUNT: usize = 4;

/// Describes how a ShaderToy channel should be populated.
#[derive(Clone, Debug)]
pub enum ChannelSource {
    Texture { path: PathBuf },
    Cubemap { directory: PathBuf },
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
    /// High-level render behaviour requested by the caller.
    pub policy: RenderPolicy,
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
            policy: RenderPolicy::default(),
        }
    }
}
