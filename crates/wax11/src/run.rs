//! Orchestrates single-shader launches by wiring CLI input, filesystem discovery, shader
//! repository resolution, and renderer setup, while delegating playlist runs to `multi.rs`
//! and surfacing diagnostics/logging policies for the daemon.
//!
//! Types:
//!
//! - None; this module ties together other crates at runtime.
//!
//! Functions:
//!
//! - `run` drives the main execution path.
//! - `initialise_tracing` configures logging.
//! - `build_client` and `prepare_single_run` resolve Shadertoy data and renderer config.
//! - Utility routines log handle decisions and translate CLI overrides into `RendererConfig`.

use anyhow::{Context, Result};
use renderer::{
    GpuMemoryMode, GpuPowerPreference, RenderMode, RenderPolicy, Renderer, RendererConfig,
};
use shadertoy::{
    load_entry_shader, ShaderHandle, ShaderRepository, ShaderSource, ShadertoyClient,
    ShadertoyConfig,
};
use tracing_subscriber::EnvFilter;

use crate::bindings::{
    channel_bindings_from_pack, map_manifest_alpha, map_manifest_color, resolve_color_space,
};
use crate::bootstrap::{
    bootstrap_filesystem, parse_surface_size, resolve_entry_handle, SingleRunConfig,
};
use crate::cli::{parse_export_format, RunArgs};
use crate::diagnostics;
use crate::handles::{EntryHandle, LaunchHandle};
use crate::multi;
use crate::paths::AppPaths;

pub fn run(args: RunArgs) -> Result<()> {
    let paths = AppPaths::discover()?;
    bootstrap_filesystem(&paths)?;
    let shader_roots = paths.shader_roots();
    let cache_root = paths.shadertoy_cache_dir();
    let (repo, resolver) = ShaderRepository::build(shader_roots.clone(), cache_root.clone())?;
    tracing::debug!(
        config = %paths.config_dir().display(),
        data = %paths.data_dir().display(),
        cache_base = %paths.cache_dir().display(),
        cache = %cache_root.display(),
        share = %paths.share_dir().display(),
        "resolved wax11 paths"
    );

    let client = build_client(&args)?;

    // Dump comprehensive startup diagnostics
    diagnostics::dump_startup_diagnostics(&args, &paths, &repo, client.as_ref(), &resolver);

    let playlist_handle = args
        .playlist
        .clone()
        .map(|arg| arg.into_inner())
        .or_else(|| {
            args.shader.clone().and_then(|arg| match arg.into_inner() {
                LaunchHandle::Playlist(handle) => Some(handle),
                LaunchHandle::Entry(_) => None,
            })
        });

    if let Some(handle) = playlist_handle {
        return multi::run_multi(
            &args,
            &repo,
            client.as_ref(),
            handle,
            &paths,
            resolver.clone(),
        );
    }

    let entry_handle = resolve_entry_handle(&args)?;
    log_entry_handle(&entry_handle, &paths);
    let handle = entry_handle.clone().into_shader_handle();
    tracing::info!(?handle, "bootstrapping wax11 wallpaper daemon");
    log_handle_warnings(&args, &handle, client.as_ref());
    let context = prepare_single_run(&args, &repo, client.as_ref(), handle.clone())?;
    run_single(context)
}

fn log_entry_handle(handle: &EntryHandle, paths: &AppPaths) {
    match handle {
        EntryHandle::RawPath(path) => {
            let resolved = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
            tracing::debug!(resource = %resolved.display(), "selected shader path");
        }
        EntryHandle::LocalPack { name } => {
            let mut searched = Vec::new();
            for root in paths.shader_roots() {
                let candidate = root.join(name);
                searched.push(candidate.display().to_string());
                if candidate.exists() {
                    let resolved = candidate
                        .canonicalize()
                        .unwrap_or_else(|_| candidate.clone());
                    tracing::debug!(resource = %resolved.display(), "selected local shader pack");
                    return;
                }
            }
            tracing::warn!(
                handle = %name,
                searched = searched.join(", "),
                "local shader pack not found in search roots"
            );
        }
        EntryHandle::Shadertoy { id } => {
            tracing::info!(shader = %id, "selected shadertoy shader");
        }
    }
}

pub fn initialise_tracing() {
    let default_filter = "warn,wax11=info,renderer=info,naga=error,wgpu=error,wgpu_core=error,wgpu_hal=error,winit=error,smithay_client_toolkit=error";
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_filter));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}

fn build_client(args: &RunArgs) -> Result<Option<ShadertoyClient>> {
    if args.cache_only {
        tracing::debug!("remote fetch disabled (--cache-only)");
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
            tracing::debug!(root = %pack.root().display(), "loaded local shader pack");
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

    let still_time = parse_still_time_arg(args.still_time.as_deref())?;
    let render_policy = if let Some(path) = args.still_export.as_ref() {
        let format = parse_export_format(path).map_err(|err| anyhow::anyhow!(err))?;
        RenderPolicy::Export {
            time: still_time,
            path: path.clone(),
            format,
        }
    } else if args.still {
        RenderPolicy::Still { time: still_time }
    } else {
        RenderPolicy::Animate {
            target_fps: match args.fps {
                Some(v) if v > 0.0 => Some(v),
                _ => None,
            },
            adaptive: args.fps_adaptive,
        }
    };

    if args.fps_adaptive && !matches!(render_policy, RenderPolicy::Animate { .. }) {
        tracing::warn!("--fps-adaptive has no effect in still/export modes");
    }

    if args.still_time.is_some() && !args.still && args.still_export.is_none() {
        anyhow::bail!("--still-time requires --still or --still-export");
    }
    let render_mode = if args.window {
        RenderMode::Windowed
    } else {
        RenderMode::Wallpaper
    };

    let render_scale = resolve_render_scale(args.render_scale)?;
    validate_occlusion_args(args.fps_adaptive, args.max_fps_occluded)?;
    let exit_on_export = args.still_exit.unwrap_or(true);
    let show_window = args.window;

    let renderer_config = RendererConfig {
        surface_size: fallback_surface,
        shader_source: shader_path,
        mode: render_mode,
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
        render_scale,
        fill_method: args.fill_method.unwrap_or_default(),
        max_fps_occluded: args.max_fps_occluded,
        show_window,
        exit_on_export,
        policy: render_policy,
        crossfade_curve: args.crossfade_curve.unwrap_or_default(),
        gpu_power: convert_gpu_power(args.gpu_power),
        gpu_memory: convert_gpu_memory(args.gpu_memory),
        gpu_latency: args.gpu_latency,
    };

    Ok(SingleRunConfig { renderer_config })
}

fn run_single(config: SingleRunConfig) -> Result<()> {
    let mut renderer = Renderer::new(config.renderer_config);
    renderer.run()
}

fn parse_still_time_arg(value: Option<&str>) -> Result<Option<f32>> {
    let Some(raw) = value else {
        return Ok(None);
    };

    let trimmed = raw.trim();
    if trimmed.is_empty() {
        anyhow::bail!("--still-time must not be empty");
    }

    if trimmed.eq_ignore_ascii_case("auto") {
        tracing::debug!("--still-time auto requested; using shader default (0s) for now");
        return Ok(None);
    }

    let seconds: f32 = trimmed.parse().map_err(|_| {
        anyhow::anyhow!("invalid --still-time value '{trimmed}'; expected seconds or 'auto'")
    })?;
    if seconds < 0.0 {
        anyhow::bail!("--still-time must be non-negative");
    }
    Ok(Some(seconds))
}

pub(crate) fn convert_gpu_power(cli_value: crate::cli::GpuPowerPreference) -> GpuPowerPreference {
    match cli_value {
        crate::cli::GpuPowerPreference::Low => GpuPowerPreference::Low,
        crate::cli::GpuPowerPreference::High => GpuPowerPreference::High,
    }
}

pub(crate) fn convert_gpu_memory(cli_value: crate::cli::GpuMemoryMode) -> GpuMemoryMode {
    match cli_value {
        crate::cli::GpuMemoryMode::Balanced => GpuMemoryMode::Balanced,
        crate::cli::GpuMemoryMode::Performance => GpuMemoryMode::Performance,
    }
}

pub(crate) fn resolve_render_scale(value: Option<f32>) -> Result<f32> {
    if let Some(scale) = value {
        if !(0.25..=1.0).contains(&scale) {
            anyhow::bail!("--render-scale must be between 0.25 and 1.0");
        }
        Ok(scale)
    } else {
        Ok(1.0)
    }
}

pub(crate) fn validate_occlusion_args(fps_adaptive: bool, max_fps: Option<f32>) -> Result<()> {
    if let Some(fps) = max_fps {
        if fps <= 0.0 {
            anyhow::bail!("--max-fps-occluded must be greater than zero");
        }
        if !fps_adaptive {
            anyhow::bail!("--max-fps-occluded requires --fps-adaptive");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_scale_validation() {
        assert_eq!(resolve_render_scale(None).unwrap(), 1.0);
        assert_eq!(resolve_render_scale(Some(0.75)).unwrap(), 0.75);
        assert!(resolve_render_scale(Some(0.1)).is_err());
        assert!(resolve_render_scale(Some(1.5)).is_err());
    }

    #[test]
    fn occlusion_validation_rules() {
        assert!(validate_occlusion_args(true, Some(10.0)).is_ok());
        assert!(validate_occlusion_args(false, Some(10.0)).is_err());
        assert!(validate_occlusion_args(true, Some(0.0)).is_err());
        assert!(validate_occlusion_args(false, None).is_ok());
    }
}
