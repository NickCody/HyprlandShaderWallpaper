use std::path::PathBuf;

use clap::{Parser, Subcommand};
use renderer::{Antialiasing, ColorSpaceMode, ExportFormat, FillMethod, ShaderCompiler};

#[derive(Parser, Debug)]
#[command(
    name = "lambdash",
    author,
    version,
    about = "Lambda Shader daemon",
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

    /// Enable playlist mode using the supplied playlist TOML file.
    #[arg(long = "playlist", alias = "multi", value_name = "FILE")]
    pub playlist: Option<PathBuf>,

    /// Render the shader in a desktop window instead of wallpaper mode.
    #[arg(long)]
    pub window: bool,

    /// Render a single still frame instead of animating continuously.
    #[arg(long)]
    pub still: bool,

    /// Timestamp (seconds or `auto`) to evaluate for still/export modes.
    #[arg(long, value_name = "SECONDS|auto")]
    pub still_time: Option<String>,

    /// Export a still frame to the provided PNG path then exit.
    #[arg(long, value_name = "PATH")]
    pub still_export: Option<PathBuf>,

    /// Control whether the process exits automatically after a still export (`true` by default).
    #[arg(long, value_name = "BOOL")]
    pub still_exit: Option<bool>,

    /// Supersampling factor to render at before presenting (0.25-1.0).
    #[arg(long, value_name = "SCALE")]
    pub render_scale: Option<f32>,

    /// How shader coordinates map to the surface (`stretch`, `center:WIDTHxHEIGHT`, `tile[:XxY]`).
    #[arg(long, value_name = "MODE", value_parser = parse_fill_method)]
    pub fill_method: Option<FillMethod>,

    /// Enable adaptive FPS throttling when the surface is occluded.
    #[arg(long)]
    pub fps_adaptive: bool,

    /// Cap FPS while occluded (requires `--fps-adaptive`).
    #[arg(long, value_name = "FPS")]
    pub max_fps_occluded: Option<f32>,

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
    /// Print resolved directories for config, data, cache, and share roots.
    Where,
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

pub fn parse_export_format(path: &PathBuf) -> Result<ExportFormat, String> {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => Ok(ExportFormat::Png),
        Some("exr") => Err("EXR export is not implemented yet; use a .png path".to_string()),
        None => Err("export path has no extension; expected .png".to_string()),
        Some(other) => Err(format!(
            "unsupported export format '.{other}'; expected .png"
        )),
    }
}

pub fn parse_fill_method(value: &str) -> Result<FillMethod, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("fill method must not be empty".into());
    }

    let (mode, rest) = match trimmed.split_once([':', '=']) {
        Some((mode, rest)) if !rest.trim().is_empty() => {
            (mode.trim().to_ascii_lowercase(), Some(rest.trim()))
        }
        _ => (trimmed.to_ascii_lowercase(), None),
    };

    match mode.as_str() {
        "stretch" => Ok(FillMethod::Stretch),
        "center" => {
            let Some(rest) = rest else {
                return Err("center fill requires dimensions (e.g. center:1920x1080)".into());
            };
            let (w, h) = parse_dimensions(rest)?;
            Ok(FillMethod::Center {
                content_width: w,
                content_height: h,
            })
        }
        "tile" => {
            let (repeat_x, repeat_y) = if let Some(rest) = rest {
                parse_repeat(rest)?
            } else {
                (1.0, 1.0)
            };
            Ok(FillMethod::Tile { repeat_x, repeat_y })
        }
        other => Err(format!(
            "unknown fill method '{other}'; expected stretch, center:WxH, or tile[:XxY]"
        )),
    }
}

fn parse_dimensions(value: &str) -> Result<(u32, u32), String> {
    let (w, h) = value
        .split_once(['x', 'X'])
        .ok_or_else(|| "expected WIDTHxHEIGHT".to_string())?;
    let width = w
        .trim()
        .parse::<u32>()
        .map_err(|_| "invalid width in center dimensions".to_string())?;
    let height = h
        .trim()
        .parse::<u32>()
        .map_err(|_| "invalid height in center dimensions".to_string())?;
    if width == 0 || height == 0 {
        return Err("center dimensions must be greater than zero".into());
    }
    Ok((width, height))
}

fn parse_repeat(value: &str) -> Result<(f32, f32), String> {
    let (x, y) = value
        .split_once(['x', 'X'])
        .ok_or_else(|| "expected repeatXxrepeatY".to_string())?;
    let repeat_x = x
        .trim()
        .parse::<f32>()
        .map_err(|_| "invalid horizontal repeat".to_string())?;
    let repeat_y = y
        .trim()
        .parse::<f32>()
        .map_err(|_| "invalid vertical repeat".to_string())?;
    if repeat_x <= 0.0 || repeat_y <= 0.0 {
        return Err("tile repeats must be positive".into());
    }
    Ok((repeat_x, repeat_y))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fill_method_variants() {
        assert_eq!(parse_fill_method("stretch").unwrap(), FillMethod::Stretch);
        assert_eq!(
            parse_fill_method("center:1920x1080").unwrap(),
            FillMethod::Center {
                content_width: 1920,
                content_height: 1080,
            }
        );
        assert!(parse_fill_method("center").is_err());
        assert_eq!(
            parse_fill_method("tile:2x3").unwrap(),
            FillMethod::Tile {
                repeat_x: 2.0,
                repeat_y: 3.0,
            }
        );
        assert_eq!(
            parse_fill_method("tile").unwrap(),
            FillMethod::Tile {
                repeat_x: 1.0,
                repeat_y: 1.0,
            }
        );
    }
}
