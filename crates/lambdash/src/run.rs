use anyhow::{Context, Result};
use renderer::{FillMethod, RenderMode, RenderPolicy, Renderer, RendererConfig};
use shadertoy::{
    load_entry_shader, ShaderHandle, ShaderRepository, ShaderSource, ShadertoyClient,
    ShadertoyConfig,
};
use tracing_subscriber::EnvFilter;

use crate::bindings::{
    channel_bindings_from_pack, map_manifest_alpha, map_manifest_color, resolve_color_space,
};
use crate::bootstrap::{
    bootstrap_filesystem, parse_surface_size, resolve_shader_handle, SingleRunConfig,
};
use crate::cli::RunArgs;
use crate::defaults::{sync_defaults, SyncOptions};
use crate::multi;
use crate::paths::AppPaths;

pub fn run(args: RunArgs) -> Result<()> {
    let paths = AppPaths::discover()?;
    let mut state = bootstrap_filesystem(&paths)?;
    let shader_roots = paths.shader_roots();
    let cache_root = paths.shadertoy_cache_dir();
    let (repo, resolver) = ShaderRepository::build(shader_roots.clone(), cache_root.clone())?;
    let previous_defaults_version = state.defaults_version.clone();
    let previous_last_sync = state.last_defaults_sync.clone();
    let defaults_report = sync_defaults(&paths, &mut state, SyncOptions::default())?;
    tracing::debug!(
        config = %paths.config_dir().display(),
        data = %paths.data_dir().display(),
        cache_base = %paths.cache_dir().display(),
        cache = %cache_root.display(),
        share = %paths.share_dir().display(),
        defaults_version = ?state.defaults_version,
        defaults_last_sync = ?state.last_defaults_sync,
        flags = ?state.flags,
        "resolved lambdash paths"
    );
    if state.defaults_version != previous_defaults_version
        || state.last_defaults_sync != previous_last_sync
    {
        state.persist(&paths.state_file())?;
    }
    if defaults_report.copied_any() {
        tracing::debug!(
            shader_packs = defaults_report.copied_shader_packs.len(),
            playlists = defaults_report.copied_playlists.len(),
            "bundled defaults installed on startup"
        );
    }
    if args.init_defaults {
        tracing::info!("--init-defaults requested; skipping daemon startup after syncing defaults");
        return Ok(());
    }

    let client = build_client(&args)?;
    if let Some(path) = args.multi.as_ref() {
        multi::run_multi(
            &args,
            &repo,
            client.as_ref(),
            path,
            &paths,
            resolver.clone(),
        )
    } else {
        let handle = resolve_shader_handle(&args, &resolver)?;
        tracing::info!(?handle, "bootstrapping lambdash wallpaper daemon");
        log_handle_warnings(&args, &handle, client.as_ref());
        let context = prepare_single_run(&args, &repo, client.as_ref(), handle.clone())?;
        run_single(context)
    }
}

pub fn initialise_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}

fn build_client(args: &RunArgs) -> Result<Option<ShadertoyClient>> {
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

fn log_handle_warnings(args: &RunArgs, handle: &ShaderHandle, client: Option<&ShadertoyClient>) {
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
    args: &RunArgs,
    repo: &ShaderRepository,
    client: Option<&ShadertoyClient>,
    handle: ShaderHandle,
) -> Result<SingleRunConfig> {
    let source = repo.resolve(&handle, client, args.refresh)?;
    let channel_report = match &source {
        ShaderSource::Local(pack) => channel_bindings_from_pack(pack),
        ShaderSource::CachedRemote(remote) => channel_bindings_from_pack(&remote.pack),
    };
    if !channel_report.issues.is_empty() {
        tracing::warn!(
            issues = channel_report.issues.len(),
            "entry pass has unsupported or missing channel bindings"
        );
        channel_report.log_warnings();
    }
    let channel_bindings = channel_report.bindings;
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
        render_scale: 1.0,
        fill_method: FillMethod::default(),
        policy: RenderPolicy::Animate {
            target_fps: match args.fps {
                Some(v) if v > 0.0 => Some(v),
                _ => None,
            },
            adaptive: false,
        },
    };

    Ok(SingleRunConfig { renderer_config })
}

fn run_single(config: SingleRunConfig) -> Result<()> {
    let mut renderer = Renderer::new(config.renderer_config);
    renderer.run()
}
