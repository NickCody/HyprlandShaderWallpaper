use anyhow::Context;
use multiconfig::MultiConfig;
use renderer::{Antialiasing, ColorSpaceMode, FillMethod, ShaderCompiler};
use shadertoy::{PathResolver, ShaderHandle, ShaderRepository, ShaderSource, ShadertoyClient};
use tracing::info;
use wgpu::TextureFormatFeatureFlags;

use crate::cli::RunArgs;
use crate::handles::{EntryHandle, PlaylistHandle};
use crate::paths::AppPaths;

/// Dumps comprehensive startup diagnostics at INFO level
pub fn dump_startup_diagnostics(
    args: &RunArgs,
    paths: &AppPaths,
    repo: &ShaderRepository,
    client: Option<&ShadertoyClient>,
    resolver: &PathResolver,
) {
    info!("=== WallShader Startup Diagnostics ===");
    
    // Global options
    dump_global_options(args, paths);
    
    // Mode and shader resolution
    if let Some(playlist_handle) = get_playlist_handle(args) {
        dump_playlist_mode(args, &playlist_handle, paths, repo, client, resolver);
    } else if let Ok(entry_handle) = crate::bootstrap::resolve_entry_handle(args) {
        dump_single_shader_mode(args, &entry_handle, paths, repo, client, resolver);
    }
    
    info!("=== End Startup Diagnostics ===");
}

fn dump_global_options(args: &RunArgs, paths: &AppPaths) {
    info!("Global Options:");
    info!("  mode: {}", if args.window { "window" } else { "desktop" });
    info!("  still: {}", args.still);
    info!("  still-time: {}", args.still_time.as_deref().unwrap_or("default"));
    info!("  still-exit: {}", args.still_exit.unwrap_or(true));
    info!("  still-export: {}", 
        args.still_export.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "none".to_string()));
    info!("  render-scale: {}", args.render_scale.unwrap_or(1.0));
    info!("  fill-method: {}", format_fill_method(args.fill_method.as_ref()));
    info!("  fps-adaptive: {}", args.fps_adaptive);
    info!("  max-fps-occluded: {}", args.max_fps_occluded.unwrap_or(0.0));
    info!("  size: {}", args.size.as_deref().unwrap_or("default"));
    info!("  fps: {}", args.fps.unwrap_or(0.0));
    info!("  refresh: {}", args.refresh);
    info!("  cache-only: {}", args.cache_only);
    info!("  antialias: {}", format_antialiasing(&args.antialias));
    info!("  shader-compiler: {}", format_shader_compiler(&args.shader_compiler));
    info!("  color-space: {}", format_color_space(&args.color_space));
    info!("  prewarm-ms: {}", args.prewarm_ms.unwrap_or(250));
    
    // Paths
    info!("Paths:");
    info!("  config: {}", paths.config_dir().display());
    info!("  data: {}", paths.data_dir().display());
    info!("  cache: {}", paths.cache_dir().display());
    info!("  shadertoy-cache: {}", paths.shadertoy_cache_dir().display());
    info!("  share: {}", paths.share_dir().display());
    info!("  shader-roots: [{}]", paths.shader_roots().iter()
        .map(|p| p.display().to_string()).collect::<Vec<_>>().join(", "));
    info!("  playlist-roots: [{}]", paths.playlist_roots().iter()
        .map(|p| p.display().to_string()).collect::<Vec<_>>().join(", "));
}

fn dump_single_shader_mode(
    args: &RunArgs,
    entry_handle: &EntryHandle,
    paths: &AppPaths,
    repo: &ShaderRepository,
    client: Option<&ShadertoyClient>,
    _resolver: &PathResolver,
) {
    info!("Single Shader Mode:");
    
    let shader_handle = entry_handle.clone().into_shader_handle();
    
    match &shader_handle {
        ShaderHandle::LocalPack(path) => {
            if path.is_absolute() {
                info!("  full-path: {}", path.display());
            } else {
                // Try to resolve against shader roots
                for root in paths.shader_roots() {
                    let candidate = root.join(path);
                    if candidate.exists() {
                        info!("  full-path: {}", candidate.canonicalize().unwrap_or(candidate).display());
                        break;
                    }
                }
            }
        }
        ShaderHandle::ShadertoyId(id) => {
            info!("  shadertoy-url: https://www.shadertoy.com/view/{}", id);
            if client.is_some() {
                info!("  shadertoy-api: available");
            } else {
                info!("  shadertoy-api: not available (cache-only)");
            }
            let cache_dir = paths.shadertoy_cache_dir().join(id);
            info!("  cache-dir: {}", cache_dir.display());
            info!("  cache-exists: {}", cache_dir.exists());
        }
    }
    
    // Try to resolve the shader to get more details
    if let Ok(source) = repo.resolve(&shader_handle, client, args.refresh) {
        match source {
            ShaderSource::Local(pack) => {
                info!("  resolved-root: {}", pack.root().display());
                info!("  shader-name: {}", pack.manifest().name.as_deref().unwrap_or("unnamed"));
            }
            ShaderSource::CachedRemote(remote) => {
                info!("  resolved-cache: {}", remote.cache_dir.display());
                info!("  resolved-root: {}", remote.pack.root().display());
                info!("  shader-name: {}", remote.pack.manifest().name.as_deref().unwrap_or("unnamed"));
                info!("  shader-id: {}", remote.id);
            }
        }
    }
}

fn dump_playlist_mode(
    _args: &RunArgs,
    playlist_handle: &PlaylistHandle,
    paths: &AppPaths,
    repo: &ShaderRepository,
    client: Option<&ShadertoyClient>,
    resolver: &PathResolver,
) {
    info!("Playlist Mode:");
    
    let playlist_roots = paths.playlist_roots();
    
    match playlist_handle.resolve_path(&playlist_roots) {
        Ok(config_path) => {
            info!("  full-path: {}", config_path.display());
            
            // Try to load and analyze the playlist
            if let Ok(contents) = std::fs::read_to_string(&config_path) {
                if let Ok(config) = MultiConfig::from_toml_str(&contents) {
                    dump_playlist_config(&config, repo, client, resolver);
                } else {
                    info!("  error: failed to parse playlist configuration");
                }
            } else {
                info!("  error: failed to read playlist file");
            }
        }
        Err(e) => {
            info!("  error: {}", e);
            info!("  searched-roots: [{}]", playlist_roots.iter()
                .map(|p| p.display().to_string()).collect::<Vec<_>>().join(", "));
        }
    }
}

fn dump_playlist_config(
    config: &MultiConfig,
    repo: &ShaderRepository,
    client: Option<&ShadertoyClient>,
    resolver: &PathResolver,
) {
    info!("Playlist Configuration:");
    
    // Global playlist settings
    info!("  version: {}", config.version);
    if let Some(default_playlist) = config.default_playlist() {
        info!("  default-playlist: {}", default_playlist);
    }
    info!("  workspace-switch-crossfade: {}ms", config.workspace_switch_crossfade().as_millis());
    
    // List all playlists
    info!("Playlists:");
    for (name, playlist) in &config.playlists {
        info!("  {}:", name);
        info!("    mode: {}", format_playlist_mode(&playlist.mode));
        info!("    item-duration: {}s", playlist.item_duration.as_secs());
        info!("    crossfade: {}ms", playlist.crossfade.as_millis());
        info!("    items: {} entries", playlist.items.len());
        
        // List items with resolved paths
        for (i, item) in playlist.items.iter().enumerate() {
            info!("      [{}] handle: {}", i, item.handle);
            
            // Try to resolve the handle
            if let Ok(entry_handle) = EntryHandle::parse_with_resolver_or_local(resolver, &item.handle) {
                let shader_handle = entry_handle.into_shader_handle();
                match repo.resolve(&shader_handle, client, false) {
                    Ok(ShaderSource::Local(pack)) => {
                        info!("      [{}] resolved-path: {}", i, pack.root().display());
                        info!("      [{}] shader-name: {}", i, pack.manifest().name.as_deref().unwrap_or("unnamed"));
                    }
                    Ok(ShaderSource::CachedRemote(remote)) => {
                        info!("      [{}] resolved-cache: {}", i, remote.cache_dir.display());
                        info!("      [{}] shader-name: {}", i, remote.pack.manifest().name.as_deref().unwrap_or("unnamed"));
                        info!("      [{}] shader-id: {}", i, remote.id);
                    }
                    Err(e) => {
                        info!("      [{}] resolution-error: {}", i, e);
                    }
                }
            } else {
                info!("      [{}] parse-error: invalid handle format", i);
            }
            
            // Item-specific settings
            if let Some(duration) = item.duration {
                info!("      [{}] duration: {}s", i, duration.as_secs());
            }
            // Items don't have individual crossfade settings - they inherit from playlist
            if let Some(fps) = item.fps {
                info!("      [{}] fps: {}", i, fps);
            }
            if let Some(antialias) = item.antialias {
                info!("      [{}] antialias: {}", i, format_multiconfig_antialias(&antialias));
            }
            if item.refresh_once {
                info!("      [{}] refresh-once: true", i);
            }
            if let Some(mode) = &item.mode {
                info!("      [{}] mode: {}", i, format_item_mode(mode));
            }
            if let Some(still_time) = item.still_time {
                info!("      [{}] still-time: {}s", i, still_time.as_secs_f32());
            }
        }
    }
    
    // Targets
    if !config.targets.is_empty() {
        info!("Targets:");
        for (selector, playlist) in &config.targets {
            info!("  {}: {}", selector, playlist);
        }
    }
}

fn get_playlist_handle(args: &RunArgs) -> Option<PlaylistHandle> {
    if let Some(playlist_arg) = &args.playlist {
        return Some(playlist_arg.0.clone());
    }
    
    if let Some(launch_arg) = &args.shader {
        if let crate::handles::LaunchHandle::Playlist(playlist_handle) = launch_arg.inner() {
            return Some(playlist_handle.clone());
        }
    }
    
    None
}

fn format_fill_method(fill_method: Option<&FillMethod>) -> String {
    match fill_method {
        Some(FillMethod::Stretch) => "stretch".to_string(),
        Some(FillMethod::Center { content_width, content_height }) => 
            format!("center:{}x{}", content_width, content_height),
        Some(FillMethod::Tile { repeat_x, repeat_y }) => 
            format!("tile:{}x{}", repeat_x, repeat_y),
        None => "stretch (default)".to_string(),
    }
}

fn format_antialiasing(antialias: &Antialiasing) -> String {
    match resolve_antialiasing_info(antialias) {
        Ok(info) => format!("{} ({}x, max {}x)", info.user_specified, info.actual_used, info.max_supported),
        Err(_) => match antialias {
            Antialiasing::Auto => "auto".to_string(),
            Antialiasing::Off => "off".to_string(),
            Antialiasing::Samples(n) => format!("{}x", n),
        }
    }
}

struct AntialiasingInfo {
    user_specified: String,
    actual_used: u32,
    max_supported: u32,
}

/// Resolves comprehensive antialiasing information: user input, actual usage, and max capability
fn resolve_antialiasing_info(antialias: &Antialiasing) -> anyhow::Result<AntialiasingInfo> {
    let (user_specified, requested_samples) = match antialias {
        Antialiasing::Auto => ("auto".to_string(), None),
        Antialiasing::Off => ("off".to_string(), Some(1)),
        Antialiasing::Samples(n) => (n.to_string(), Some(*n)),
    };
    
    let gpu_info = resolve_gpu_antialiasing_capabilities()?;
    
    let actual_used = if let Some(requested) = requested_samples {
        // Apply the same resolution logic as the renderer for explicit requests
        if requested == 1 {
            1 // Off always resolves to 1
        } else if gpu_info.supported_samples.contains(&requested) {
            requested
        } else {
            // Find the highest supported value that's <= requested
            gpu_info.supported_samples
                .iter()
                .copied()
                .filter(|&count| count <= requested)
                .max()
                .unwrap_or(1)
        }
    } else {
        // Auto mode - use max supported
        gpu_info.max_supported
    };
    
    // Apply the same constraints as the renderer
    let final_used = apply_renderer_constraints(actual_used, &gpu_info);
    
    Ok(AntialiasingInfo {
        user_specified,
        actual_used: final_used,
        max_supported: gpu_info.max_supported,
    })
}

struct GpuAntialiasingCapabilities {
    supported_samples: Vec<u32>,
    max_supported: u32,
    is_software: bool,
    has_msaa_resolve: bool,
    has_high_sample_features: bool,
}

/// Performs a minimal GPU probe to determine antialiasing capabilities
fn resolve_gpu_antialiasing_capabilities() -> anyhow::Result<GpuAntialiasingCapabilities> {
    // Create a minimal wgpu instance just for probing capabilities
    let instance = wgpu::Instance::default();
    
    // Request any adapter for capability detection
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None, // We don't need a surface for this probe
        force_fallback_adapter: false,
    }))
    .context("No suitable GPU adapter found for antialiasing detection")?;
    
    let adapter_info = adapter.get_info();
    let is_software = matches!(adapter_info.device_type, wgpu::DeviceType::Cpu)
        || adapter_info.name.to_ascii_lowercase().contains("llvmpipe")
        || adapter_info.name.to_ascii_lowercase().contains("softpipe")
        || adapter_info.driver.to_ascii_lowercase().contains("llvmpipe")
        || adapter_info.driver.to_ascii_lowercase().contains("softpipe");
    
    // Use a common surface format for capability detection
    let surface_format = wgpu::TextureFormat::Bgra8UnormSrgb;
    let format_features = adapter.get_texture_format_features(surface_format);
    let mut supported_samples = format_features.flags.supported_sample_counts();
    
    if !supported_samples.contains(&1) {
        supported_samples.push(1);
    }
    supported_samples.sort_unstable();
    supported_samples.dedup();
    
    let has_msaa_resolve = format_features
        .flags
        .contains(TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE);
    
    let adapter_features = adapter.features();
    let has_high_sample_features = adapter_features.contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES);
    
    // Determine the theoretical maximum supported
    let max_supported = *supported_samples.last().unwrap_or(&1);
    
    Ok(GpuAntialiasingCapabilities {
        supported_samples,
        max_supported,
        is_software,
        has_msaa_resolve,
        has_high_sample_features,
    })
}

/// Applies the same constraints as the renderer to determine actual usage
fn apply_renderer_constraints(requested: u32, gpu_info: &GpuAntialiasingCapabilities) -> u32 {
    let mut sample_count = requested;
    
    // Check if MSAA resolve is supported
    if sample_count > 1 && !gpu_info.has_msaa_resolve {
        sample_count = 1;
    }
    
    // Disable MSAA on software rasterizers for performance
    if gpu_info.is_software && sample_count > 1 {
        sample_count = 1;
    }
    
    // Check for high sample count feature support
    if sample_count > 4 && !gpu_info.has_high_sample_features {
        let fallback = gpu_info.supported_samples
            .iter()
            .copied()
            .filter(|&count| count <= 4)
            .max()
            .unwrap_or(1);
        sample_count = fallback;
    }
    
    sample_count
}

fn format_multiconfig_antialias(antialias: &multiconfig::AntialiasSetting) -> String {
    match antialias {
        multiconfig::AntialiasSetting::Auto => "auto".to_string(),
        multiconfig::AntialiasSetting::Off => "off".to_string(),
        multiconfig::AntialiasSetting::Samples2 => "2x".to_string(),
        multiconfig::AntialiasSetting::Samples4 => "4x".to_string(),
        multiconfig::AntialiasSetting::Samples8 => "8x".to_string(),
        multiconfig::AntialiasSetting::Samples16 => "16x".to_string(),
    }
}

fn format_shader_compiler(compiler: &ShaderCompiler) -> String {
    match compiler {
        ShaderCompiler::Shaderc => "shaderc".to_string(),
        ShaderCompiler::NagaGlsl => "naga".to_string(),
    }
}

fn format_color_space(color_space: &ColorSpaceMode) -> String {
    match color_space {
        ColorSpaceMode::Auto => "auto".to_string(),
        ColorSpaceMode::Gamma => "gamma".to_string(),
        ColorSpaceMode::Linear => "linear".to_string(),
    }
}

fn format_playlist_mode(mode: &multiconfig::PlaylistMode) -> String {
    match mode {
        multiconfig::PlaylistMode::Continuous => "continuous".to_string(),
        multiconfig::PlaylistMode::Shuffle => "shuffle".to_string(),
    }
}

fn format_item_mode(mode: &multiconfig::PlaylistItemMode) -> String {
    match mode {
        multiconfig::PlaylistItemMode::Animate => "animate".to_string(),
        multiconfig::PlaylistItemMode::Still => "still".to_string(),
    }
}