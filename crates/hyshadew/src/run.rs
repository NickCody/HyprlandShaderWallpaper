use anyhow::{Context, Result};
use renderer::{RenderMode, Renderer, RendererConfig};
use shadertoy::{
    load_entry_shader, ShaderHandle, ShaderRepository, ShaderSource, ShadertoyClient,
    ShadertoyConfig,
};
use tracing_subscriber::EnvFilter;

use crate::bindings::{
    channel_bindings_from_pack, map_manifest_alpha, map_manifest_color, resolve_color_space,
};
use crate::bootstrap::{parse_surface_size, resolve_shader_handle, SingleRunConfig};
use crate::cli::Args;
use crate::multi;
use crate::paths::AppPaths;

pub fn run(args: Args) -> Result<()> {
    initialise_tracing();

    let paths = AppPaths::discover()?;
    let shader_roots = paths.shader_roots();
    let cache_root = paths.shadertoy_cache_dir();
    tracing::debug!(
        config = %paths.config_dir().display(),
        data = %paths.data_dir().display(),
        cache_base = %paths.cache_dir().display(),
        cache = %cache_root.display(),
        share = %paths.share_dir().display(),
        "resolved hyshadew paths"
    );
    let repo = ShaderRepository::new(shader_roots, cache_root);
    let client = build_client(&args)?;
    if let Some(path) = args.multi.as_ref() {
        multi::run_multi(&args, &repo, client.as_ref(), path, &paths)
    } else {
        let handle = resolve_shader_handle(&args)?;
        tracing::info!(?handle, "bootstrapping hyshadew wallpaper daemon");
        log_handle_warnings(&args, &handle, client.as_ref());
        let context = prepare_single_run(&args, &repo, client.as_ref(), handle.clone())?;
        run_single(context)
    }
}

fn initialise_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}

fn build_client(args: &Args) -> Result<Option<ShadertoyClient>> {
    if args.cache_only {
        tracing::info!("remote fetch disabled (--cache-only)");
        return Ok(None);
    }

    if let Some(ref key) = args.shadertoy_api_key {
        let config = ShadertoyConfig::new(key.as_str())
            .context("invalid Shadertoy API key configuration")?;
        Ok(Some(
            ShadertoyClient::new(config).context("failed to construct Shadertoy client")?,
        ))
    } else {
        Ok(None)
    }
}

fn log_handle_warnings(args: &Args, handle: &ShaderHandle, client: Option<&ShadertoyClient>) {
    if matches!(handle, ShaderHandle::ShadertoyId(_)) {
        if args.refresh && (args.cache_only || client.is_none()) {
            tracing::warn!("refresh requested but no Shadertoy client available; using cache only");
        }
        if !args.cache_only && client.is_none() {
            tracing::info!(
                "Shadertoy API key not provided; relying on cached shader data if present"
            );
        }
    }
}

fn prepare_single_run(
    args: &Args,
    repo: &ShaderRepository,
    client: Option<&ShadertoyClient>,
    handle: ShaderHandle,
) -> Result<SingleRunConfig> {
    let source = repo.resolve(&handle, client, args.refresh)?;
    let channel_bindings = match &source {
        ShaderSource::Local(pack) => channel_bindings_from_pack(pack),
        ShaderSource::CachedRemote(remote) => channel_bindings_from_pack(&remote.pack),
    };
    let (surface_alpha, manifest_color) = match &source {
        ShaderSource::Local(pack) => (
            map_manifest_alpha(pack.manifest().surface_alpha),
            map_manifest_color(pack.manifest().color_space),
        ),
        ShaderSource::CachedRemote(remote) => (
            map_manifest_alpha(remote.pack.manifest().surface_alpha),
            map_manifest_color(remote.pack.manifest().color_space),
        ),
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

    let color_space = resolve_color_space(args.color_space, manifest_color);

    let renderer_config = RendererConfig {
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
        shader_compiler: args.shader_compiler,
        color_space,
    };

    Ok(SingleRunConfig { renderer_config })
}

fn run_single(config: SingleRunConfig) -> Result<()> {
    let mut renderer = Renderer::new(config.renderer_config);
    renderer.run()
}
