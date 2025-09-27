use anyhow::{bail, Context, Result};
use clap::Parser;
use renderer::{
    Antialiasing, ChannelBindings, RenderMode, Renderer, RendererConfig,
    SurfaceAlpha as RendererSurfaceAlpha,
};
use shadertoy::{
    load_entry_shader, InputSource, LocalPack, ShaderHandle, ShaderRepository, ShaderSource,
    ShadertoyClient, ShadertoyConfig, SurfaceAlpha as ManifestSurfaceAlpha,
};
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(
    name = "hyshadew",
    author,
    version,
    about = "Hyprland Shader Wallpaper daemon"
)]
struct Args {
    /// Shader handle (e.g. `shadertoy://abc123` or `local-shaders/demo`)
    #[arg(value_name = "HANDLE")]
    shader: Option<String>,

    /// Convenience flag for specifying a Shadertoy URL or ID.
    #[arg(long, value_name = "URL")]
    shadertoy: Option<String>,

    /// Render the shader in a desktop window instead of wallpaper mode.
    #[arg(long)]
    window: bool,

    /// Override the render resolution (e.g. `1280x720`).
    #[arg(long, value_name = "WIDTHxHEIGHT")]
    size: Option<String>,

    /// Optional FPS cap for wallpaper rendering (0=uncapped).
    #[arg(long, value_name = "FPS")]
    fps: Option<f32>,

    /// Force refresh of the remote shader cache before launch.
    #[arg(long)]
    refresh: bool,

    /// Skip any remote fetches, even if an API key is available.
    #[arg(long)]
    cache_only: bool,

    /// Shadertoy API key; can also be supplied via the `SHADERTOY_API_KEY` env var.
    #[arg(long, env = "SHADERTOY_API_KEY")]
    shadertoy_api_key: Option<String>,

    /// Anti-aliasing policy: `auto`, `off`, or an explicit MSAA sample count (e.g. `4`).
    #[arg(long, value_name = "MODE", value_parser = parse_antialias, default_value = "auto")]
    antialias: Antialiasing,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    // Initialise structured logging, respecting `RUST_LOG` overrides and
    // omitting target/module labels to keep the daemon logs succinct.
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();

    let repo = ShaderRepository::with_defaults();
    let handle = resolve_shader_handle(&args)?;
    tracing::info!(?handle, "bootstrapping hyshadew wallpaper daemon");

    let client = if !args.cache_only {
        if let Some(ref key) = args.shadertoy_api_key {
            let config = ShadertoyConfig::new(key.as_str())
                .context("invalid Shadertoy API key configuration")?;
            Some(ShadertoyClient::new(config).context("failed to construct Shadertoy client")?)
        } else {
            None
        }
    } else {
        tracing::info!("remote fetch disabled (--cache-only)");
        None
    };

    if matches!(&handle, ShaderHandle::ShadertoyId(_)) {
        if args.refresh && (args.cache_only || client.is_none()) {
            tracing::warn!("refresh requested but no Shadertoy client available; using cache only");
        }
        if !args.cache_only && client.is_none() {
            tracing::info!(
                "Shadertoy API key not provided; relying on cached shader data if present"
            );
        }
    }

    let source = repo.resolve(&handle, client.as_ref(), args.refresh)?;
    let channel_bindings = match &source {
        ShaderSource::Local(pack) => channel_bindings_from_pack(pack),
        ShaderSource::CachedRemote(remote) => channel_bindings_from_pack(&remote.pack),
    };
    let surface_alpha = match &source {
        ShaderSource::Local(pack) => map_manifest_alpha(pack.manifest().surface_alpha),
        ShaderSource::CachedRemote(remote) => {
            map_manifest_alpha(remote.pack.manifest().surface_alpha)
        }
    };
    let shader_path = load_entry_shader(&source)?;

    match &source {
        ShaderSource::Local(pack) => {
            tracing::info!(root = %pack.root().display(), "loaded local shader pack");
        }
        ShaderSource::CachedRemote(remote) => {
            tracing::info!(
                shader = %remote.id,
                cache = %remote.cache_dir.display(),
                root = %remote.pack.root().display(),
                refreshed = args.refresh,
                "loaded Shadertoy shader"
            );
        }
    }

    tracing::debug!(path = %shader_path.display(), window = args.window, "resolved entry shader");

    let requested_size = args
        .size
        .as_ref()
        .map(|value| parse_surface_size(value))
        .transpose()?;

    let fallback_surface = requested_size.unwrap_or((1920, 1080));

    let config = RendererConfig {
        surface_size: fallback_surface,
        shader_source: shader_path,
        mode: if args.window {
            tracing::info!("windowed rendering mode requested (placeholder implementation)");
            RenderMode::Windowed
        } else {
            RenderMode::Wallpaper
        },
        requested_size,
        target_fps: match args.fps {
            Some(v) if v > 0.0 => Some(v),
            _ => None,
        },
        channel_bindings,
        antialiasing: args.antialias,
        surface_alpha,
    };

    let mut renderer = Renderer::new(config);
    renderer.run()
}

fn channel_bindings_from_pack(pack: &LocalPack) -> ChannelBindings {
    let mut bindings = ChannelBindings::default();
    let manifest = pack.manifest();
    let entry_name = &manifest.entry;
    let entry_pass = manifest.passes.iter().find(|pass| &pass.name == entry_name);

    let Some(pass) = entry_pass else {
        tracing::warn!(entry = %entry_name, "entry pass missing; no channels bound");
        return bindings;
    };

    for input in &pass.inputs {
        match &input.source {
            InputSource::Texture { path } => {
                let resolved = if path.is_absolute() {
                    path.clone()
                } else {
                    pack.root().join(path)
                };
                let resolved_for_log = resolved.clone();
                if !resolved_for_log.exists() {
                    tracing::warn!(
                        channel = input.channel,
                        path = %resolved_for_log.display(),
                        "channel texture not found on disk"
                    );
                }
                if let Err(err) = bindings.set_texture(input.channel as usize, resolved) {
                    tracing::warn!(
                        channel = input.channel,
                        path = %resolved_for_log.display(),
                        error = %err,
                        "failed to register texture channel"
                    );
                }
            }
            InputSource::Buffer { name } => {
                tracing::warn!(
                    channel = input.channel,
                    buffer = %name,
                    "buffer channels are not supported yet"
                );
            }
            InputSource::Cubemap { directory } => {
                tracing::warn!(
                    channel = input.channel,
                    dir = %directory.display(),
                    "cubemap channels are not supported yet"
                );
            }
            InputSource::Audio { path } => {
                tracing::warn!(
                    channel = input.channel,
                    path = %path.display(),
                    "audio channels are not supported yet"
                );
            }
        }
    }

    bindings
}

fn map_manifest_alpha(alpha: ManifestSurfaceAlpha) -> RendererSurfaceAlpha {
    match alpha {
        ManifestSurfaceAlpha::Opaque => RendererSurfaceAlpha::Opaque,
        ManifestSurfaceAlpha::Transparent => RendererSurfaceAlpha::Transparent,
    }
}

fn resolve_shader_handle(args: &Args) -> Result<ShaderHandle> {
    if let Some(ref shadertoy) = args.shadertoy {
        let normalized = normalize_shadertoy_reference(shadertoy)?;
        return Ok(ShaderHandle::from_input(&normalized));
    }

    let raw = args
        .shader
        .as_ref()
        .cloned()
        .unwrap_or_else(|| "local-demo".to_string());
    Ok(ShaderHandle::from_input(&raw))
}

fn normalize_shadertoy_reference(input: &str) -> Result<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        bail!("shadertoy reference must not be empty");
    }

    if let Some(rest) = trimmed.strip_prefix("shadertoy://") {
        let id = rest.trim();
        if id.is_empty() {
            bail!("shadertoy identifier missing after scheme");
        }
        return Ok(format!("shadertoy://{}", id));
    }

    if trimmed.contains("shadertoy.com") {
        let id_candidate = trimmed
            .split('/')
            .rev()
            .find(|segment| !segment.is_empty())
            .map(|segment| {
                segment
                    .split(|c| c == '?' || c == '#')
                    .next()
                    .unwrap_or(segment)
            })
            .filter(|id| !id.is_empty());

        if let Some(id) = id_candidate {
            return Ok(format!("shadertoy://{}", id));
        } else {
            bail!("unable to extract shader id from shadertoy url");
        }
    }

    Ok(format!("shadertoy://{}", trimmed))
}

fn parse_surface_size(spec: &str) -> Result<(u32, u32)> {
    let trimmed = spec.trim();
    let (width, height) = trimmed
        .split_once(['x', 'X', '×'])
        .ok_or_else(|| anyhow::anyhow!("expected WxH format, e.g. 1920x1080"))?;

    let width: u32 = width
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid width in size specification"))?;
    let height: u32 = height
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid height in size specification"))?;

    if width == 0 || height == 0 {
        anyhow::bail!("surface dimensions must be greater than zero");
    }

    Ok((width, height))
}

fn parse_antialias(value: &str) -> Result<Antialiasing, String> {
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

            if samples == 0 {
                return Ok(Antialiasing::Off);
            }

            if samples == 1 {
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
