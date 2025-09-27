use clap::Parser;
use renderer::Antialiasing;

#[derive(Parser, Debug)]
#[command(
    name = "hyshadew",
    author,
    version,
    about = "Hyprland Shader Wallpaper daemon"
)]
pub struct Args {
    /// Shader handle (e.g. `shadertoy://abc123` or `local-shaders/demo`)
    #[arg(value_name = "HANDLE")]
    pub shader: Option<String>,

    /// Convenience flag for specifying a Shadertoy URL or ID.
    #[arg(long, value_name = "URL")]
    pub shadertoy: Option<String>,

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
}

pub fn parse() -> Args {
    Args::parse()
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
