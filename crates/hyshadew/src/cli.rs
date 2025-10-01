use std::path::PathBuf;

use clap::{Parser, Subcommand};
use renderer::{Antialiasing, ColorSpaceMode, ShaderCompiler};

#[derive(Parser, Debug)]
#[command(
    name = "hyshadew",
    author,
    version,
    about = "Hyprland Shader Wallpaper daemon",
    arg_required_else_help = false
)]
pub struct Cli {
    #[command(flatten)]
    pub run: RunArgs,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Parser, Debug)]
pub struct RunArgs {
    /// Shader handle (e.g. `shadertoy://abc123` or `local-shaders/demo`)
    #[arg(value_name = "HANDLE")]
    pub shader: Option<String>,

    /// Convenience flag for specifying a Shadertoy URL or ID.
    #[arg(long, value_name = "URL")]
    pub shadertoy: Option<String>,

    /// Enable playlist mode using the supplied multi-config TOML file or directory.
    #[arg(long, value_name = "PATH")]
    pub multi: Option<PathBuf>,

    /// Render the shader in a desktop window instead of wallpaper mode.
    #[arg(long)]
    pub window: bool,

    /// Override the render resolution (e.g. `1280x720`).
    #[arg(long, value_name = "WIDTHxHEIGHT")]
    pub size: Option<String>,

    /// Optional FPS cap for wallpaper rendering (0=uncapped).
    #[arg(long, value_name = "FPS")]
    pub fps: Option<f32>,

    /// Force refresh of the remote shader cache before launch.
    #[arg(long)]
    pub refresh: bool,

    /// Skip any remote fetches, even if an API key is available.
    #[arg(long)]
    pub cache_only: bool,

    /// Shadertoy API key; can also be supplied via the `SHADERTOY_API_KEY` env var.
    #[arg(long, env = "SHADERTOY_API_KEY")]
    pub shadertoy_api_key: Option<String>,

    /// Anti-aliasing policy: `auto`, `off`, or an explicit MSAA sample count (e.g. `4`).
    #[arg(
        long,
        value_name = "MODE",
        value_parser = parse_antialias,
        default_value = "auto"
    )]
    pub antialias: Antialiasing,

    /// Shader compiler backend: `shaderc` (default) or `naga`.
    #[arg(
        long,
        value_name = "COMPILER",
        value_parser = parse_shader_compiler,
        default_value_t = ShaderCompiler::default()
    )]
    pub shader_compiler: ShaderCompiler,

    /// Output color space handling: `auto`, `gamma`, or `linear`.
    #[arg(
        long,
        value_name = "MODE",
        value_parser = parse_color_space,
        default_value = "auto"
    )]
    pub color_space: ColorSpaceMode,

    /// Warm-up interval (ms) to pre-render the next shader before crossfade.
    #[arg(long, value_name = "MILLISECONDS")]
    pub prewarm_ms: Option<u64>,

    /// Initialise defaults (creates directories, installs bundled content) then exit.
    #[arg(long)]
    pub init_defaults: bool,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Manage bundled defaults (shader packs, playlists, paths).
    Defaults(DefaultsCommand),
}

#[derive(Parser, Debug)]
pub struct DefaultsCommand {
    #[command(subcommand)]
    pub action: DefaultsAction,
}

#[derive(Subcommand, Debug)]
pub enum DefaultsAction {
    /// Copy bundled defaults into user directories.
    Sync(DefaultsSyncArgs),
    /// Show bundled defaults and whether they exist locally.
    List,
    /// Print resolved directories for config, data, cache, and share roots.
    Where,
}

#[derive(Parser, Debug, Default)]
pub struct DefaultsSyncArgs {
    /// Preview which defaults would be copied without writing to disk.
    #[arg(long)]
    pub dry_run: bool,
}

pub fn parse() -> Cli {
    Cli::parse()
}

pub fn parse_antialias(value: &str) -> Result<Antialiasing, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("anti-alias mode must not be empty".to_string());
    }

    let normalized = trimmed.to_ascii_lowercase();
    match normalized.as_str() {
        "auto" | "max" | "default" => Ok(Antialiasing::Auto),
        "off" | "none" | "disable" | "disabled" | "0" => Ok(Antialiasing::Off),
        _ => {
            let samples: u32 = normalized.parse().map_err(|_| {
                format!("invalid anti-alias sample count '{trimmed}'; use auto/off or 2/4/8/16")
            })?;

            if samples == 0 || samples == 1 {
                return Ok(Antialiasing::Off);
            }

            if !matches!(samples, 2 | 4 | 8 | 16) {
                return Err(format!(
                    "unsupported sample count {samples}; supported values are 2, 4, 8, or 16"
                ));
            }

            Ok(Antialiasing::Samples(samples))
        }
    }
}

pub fn parse_shader_compiler(value: &str) -> Result<ShaderCompiler, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("shader compiler must not be empty".to_string());
    }

    let normalized = trimmed.to_ascii_lowercase();
    match normalized.as_str() {
        "shaderc" => {
            if cfg!(feature = "shaderc") {
                Ok(ShaderCompiler::Shaderc)
            } else {
                Err("shaderc support is not enabled in this build".to_string())
            }
        }
        "naga" | "naga-glsl" => Ok(ShaderCompiler::NagaGlsl),
        _ => Err("unknown shader compiler (expected shaderc or naga)".to_string()),
    }
}

pub fn parse_color_space(value: &str) -> Result<ColorSpaceMode, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("color space must not be empty".to_string());
    }

    let normalized = trimmed.to_ascii_lowercase();
    match normalized.as_str() {
        "auto" => Ok(ColorSpaceMode::Auto),
        "gamma" | "srgb-off" | "shadertoy" => Ok(ColorSpaceMode::Gamma),
        "linear" | "srgb" => Ok(ColorSpaceMode::Linear),
        other => Err(format!(
            "unknown color space '{other}'; expected auto, gamma, or linear"
        )),
    }
}
