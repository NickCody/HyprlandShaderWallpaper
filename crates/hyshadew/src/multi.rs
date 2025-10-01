use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use multiconfig::{AntialiasSetting, MultiConfig};
use renderer::{
    Antialiasing, ChannelBindings, ColorSpaceMode, OutputId, RenderMode, RendererConfig,
    SurfaceAlpha, SurfaceId, SurfaceInfo, SurfaceSelector, SwapRequest, WallpaperRuntime,
    WindowRuntime,
};
use scheduler::{ScheduledItem, Scheduler, TargetId};
use serde::Deserialize;
use shadertoy::{load_entry_shader, ShaderHandle, ShaderRepository, ShaderSource, ShadertoyClient};
use tracing::{debug, error, info, trace, warn};

use crate::bindings::{
    channel_bindings_from_pack, map_manifest_alpha, map_manifest_color, resolve_color_space,
};
use crate::bootstrap::parse_surface_size;
use crate::cli::RunArgs;
use crate::paths::AppPaths;

const DEFAULT_PREWARM_MS: u64 = 250;

pub fn run_multi(
    args: &RunArgs,
    repo: &ShaderRepository,
    client: Option<&ShadertoyClient>,
    path: &Path,
    paths: &AppPaths,
) -> Result<()> {
    let (config, config_path) = load_config(path)?;
    info!(config = %config_path.display(), "loaded multi-playlist configuration");
    let playlist_roots = paths.playlist_roots();
    let playlist_user_dirs = paths.playlist_user_dirs();
    debug!(
        roots = ?playlist_roots,
        user_dirs = ?playlist_user_dirs,
        "multi playlist search roots"
    );

    if args.window {
        run_window_multi(args, repo, client, config)
    } else {
        run_wallpaper_multi(args, repo, client, config)
    }
}

fn load_config(path: &Path) -> Result<(MultiConfig, PathBuf)> {
    let resolved = if path.is_dir() {
        path.join("default.toml")
    } else {
        path.to_path_buf()
    };
    let contents = fs::read_to_string(&resolved)
        .with_context(|| format!("failed to read multi config at {}", resolved.display()))?;
    let config = MultiConfig::from_toml_str(&contents)?;
    Ok((config, resolved))
}

fn run_wallpaper_multi(
    args: &RunArgs,
    repo: &ShaderRepository,
    client: Option<&ShadertoyClient>,
    config: MultiConfig,
) -> Result<()> {
    let requested_size = args
        .size
        .as_ref()
        .map(|value| parse_surface_size(value))
        .transpose()?;
    let fallback_surface = requested_size.unwrap_or((1920, 1080));

    let mut cache = ShaderCache::new(repo, client, args.cache_only);
    let bootstrap = select_bootstrap_shader(&config, &mut cache)?;

    let bootstrap_color = resolve_color_space(args.color_space, bootstrap.color_space);

    let renderer_config = RendererConfig {
        surface_size: fallback_surface,
        shader_source: bootstrap.shader_path.clone(),
        mode: RenderMode::Wallpaper,
        requested_size,
        target_fps: normalize_fps(args.fps),
        channel_bindings: bootstrap.channel_bindings.clone(),
        antialiasing: args.antialias,
        surface_alpha: bootstrap.surface_alpha,
        color_space: bootstrap_color,
        shader_compiler: args.shader_compiler,
    };

    let runtime = WallpaperRuntime::spawn(renderer_config)?;

    let seed = seed_from_time();
    let options = EngineOptions {
        cache_only: args.cache_only,
        refresh_all: args.refresh,
        global_fps: normalize_fps(args.fps),
        global_antialias: args.antialias,
        global_color_space: args.color_space,
        prewarm: Duration::from_millis(args.prewarm_ms.unwrap_or(DEFAULT_PREWARM_MS)),
    };
    let mut engine = PlaylistEngine::new(config, cache, seed, options);

    main_wallpaper_loop(runtime, &mut engine)
}

fn run_window_multi(
    args: &RunArgs,
    repo: &ShaderRepository,
    client: Option<&ShadertoyClient>,
    config: MultiConfig,
) -> Result<()> {
    if config.default_playlist().is_none() {
        bail!("window mode requires defaults.playlist to be set");
    }

    let requested_size = args
        .size
        .as_ref()
        .map(|value| parse_surface_size(value))
        .transpose()?;
    let fallback_surface = requested_size.unwrap_or((1280, 720));

    let mut cache = ShaderCache::new(repo, client, args.cache_only);
    let bootstrap = select_bootstrap_shader(&config, &mut cache)?;

    let bootstrap_color = resolve_color_space(args.color_space, bootstrap.color_space);

    let renderer_config = RendererConfig {
        surface_size: fallback_surface,
        shader_source: bootstrap.shader_path.clone(),
        mode: RenderMode::Windowed,
        requested_size,
        target_fps: None,
        channel_bindings: bootstrap.channel_bindings.clone(),
        antialiasing: args.antialias,
        surface_alpha: bootstrap.surface_alpha,
        color_space: bootstrap_color,
        shader_compiler: args.shader_compiler,
    };

    let runtime = WindowRuntime::spawn(renderer_config)?;

    let seed = seed_from_time();
    let options = EngineOptions {
        cache_only: args.cache_only,
        refresh_all: args.refresh,
        global_fps: None,
        global_antialias: args.antialias,
        global_color_space: args.color_space,
        prewarm: Duration::from_millis(args.prewarm_ms.unwrap_or(DEFAULT_PREWARM_MS)),
    };
    let mut engine = PlaylistEngine::new(config, cache, seed, options);

    let surface = SurfaceInfo {
        surface_id: SurfaceId::from(1u64),
        output_id: None,
        output_name: None,
        size: requested_size,
    };
    let surface_id = surface.surface_id;

    let initial_actions = engine.sync_targets(&[surface], None, Instant::now())?;
    if !apply_window_actions(&runtime, initial_actions)? {
        return runtime.shutdown();
    }

    'main: loop {
        let pending_advances = runtime.take_advance_requests();
        if pending_advances > 0 {
            info!(
                requests = pending_advances,
                "window advance requested via spacebar"
            );
        }
        for _ in 0..pending_advances {
            let actions = engine.skip_surface(surface_id, Instant::now())?;
            if !apply_window_actions(&runtime, actions)? {
                break 'main;
            }
        }

        std::thread::sleep(Duration::from_millis(120));
        let actions = engine.tick(Instant::now(), None)?;
        if !apply_window_actions(&runtime, actions)? {
            break;
        }
    }

    runtime.shutdown()
}

fn main_wallpaper_loop(runtime: WallpaperRuntime, engine: &mut PlaylistEngine<'_>) -> Result<()> {
    let mut hypr = HyprlandResolver::new();
    loop {
        let now = Instant::now();
        let hypr_snapshot = hypr.snapshot();
        let surfaces = runtime
            .surfaces()
            .context("failed to query wallpaper surfaces")?;
        let actions = engine.sync_targets(&surfaces, hypr_snapshot.as_ref(), now)?;
        apply_actions(&runtime, actions)?;

        let tick_actions = engine.tick(now, hypr_snapshot.as_ref())?;
        apply_actions(&runtime, tick_actions)?;

        std::thread::sleep(Duration::from_millis(100));
    }
}

fn apply_actions(runtime: &WallpaperRuntime, actions: Vec<SwapAction>) -> Result<()> {
    for action in actions {
        info!(
            target = %action.target_display,
            handle = %action.handle,
            crossfade_ms = action.request.crossfade.as_millis(),
            warmup_ms = action.request.warmup.as_millis(),
            fps = action.request.target_fps,
            antialias = ?action.request.antialiasing,
            "swapping shader"
        );
        if let Err(err) = runtime.swap_shader(action.selector, action.request) {
            error!(target = %action.target_display, error = ?err, "failed to swap shader");
        }
    }
    Ok(())
}

fn apply_window_actions(runtime: &WindowRuntime, actions: Vec<SwapAction>) -> Result<bool> {
    for action in actions {
        info!(
            target = %action.target_display,
            handle = %action.handle,
            crossfade_ms = action.request.crossfade.as_millis(),
            warmup_ms = action.request.warmup.as_millis(),
            fps = action.request.target_fps,
            antialias = ?action.request.antialiasing,
            "swapping shader"
        );
        let SwapRequest {
            shader_source,
            channel_bindings,
            crossfade,
            antialiasing,
            warmup,
            ..
        } = action.request;
        if let Err(err) = runtime.swap_shader(
            shader_source,
            channel_bindings,
            antialiasing,
            crossfade,
            warmup,
        ) {
            error!(target = %action.target_display, error = ?err, "failed to swap shader");
            return Ok(false);
        }
    }
    Ok(true)
}

fn normalize_fps(value: Option<f32>) -> Option<f32> {
    value.and_then(|fps| if fps > 0.0 { Some(fps) } else { None })
}

fn seed_from_time() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(1)
}

struct EngineOptions {
    cache_only: bool,
    refresh_all: bool,
    global_fps: Option<f32>,
    global_antialias: Antialiasing,
    global_color_space: ColorSpaceMode,
    prewarm: Duration,
}

struct ShaderAssets {
    shader_path: PathBuf,
    channel_bindings: ChannelBindings,
    surface_alpha: SurfaceAlpha,
    color_space: ColorSpaceMode,
}

#[derive(Clone)]
struct CachedShader {
    shader_path: PathBuf,
    channel_bindings: ChannelBindings,
    surface_alpha: SurfaceAlpha,
    color_space: ColorSpaceMode,
}

struct ShaderCache<'a> {
    repo: &'a ShaderRepository,
    client: Option<&'a ShadertoyClient>,
    cache_only: bool,
    entries: HashMap<String, CachedShader>,
}

impl<'a> ShaderCache<'a> {
    fn new(
        repo: &'a ShaderRepository,
        client: Option<&'a ShadertoyClient>,
        cache_only: bool,
    ) -> Self {
        Self {
            repo,
            client,
            cache_only,
            entries: HashMap::new(),
        }
    }

    fn resolve(&mut self, handle: &str, refresh: bool) -> Result<ShaderAssets> {
        if !refresh {
            if let Some(cached) = self.entries.get(handle) {
                debug!(handle, "using cached shader assets");
                return Ok(ShaderAssets {
                    shader_path: cached.shader_path.clone(),
                    channel_bindings: cached.channel_bindings.clone(),
                    surface_alpha: cached.surface_alpha,
                    color_space: cached.color_space,
                });
            }
        }

        let shader_handle = ShaderHandle::from_input(handle);
        debug!(handle = %handle, refresh, "resolving shader handle");
        let source = self
            .repo
            .resolve(&shader_handle, self.client, refresh && !self.cache_only)
            .with_context(|| format!("failed to resolve shader '{handle}'"))?;
        let (channel_bindings, surface_alpha, color_space) = match &source {
            ShaderSource::Local(pack) => (
                channel_bindings_from_pack(pack),
                map_manifest_alpha(pack.manifest().surface_alpha),
                map_manifest_color(pack.manifest().color_space),
            ),
            ShaderSource::CachedRemote(remote) => (
                channel_bindings_from_pack(&remote.pack),
                map_manifest_alpha(remote.pack.manifest().surface_alpha),
                map_manifest_color(remote.pack.manifest().color_space),
            ),
        };
        let shader_path = load_entry_shader(&source)?;
        self.entries.insert(
            handle.to_string(),
            CachedShader {
                shader_path: shader_path.clone(),
                channel_bindings: channel_bindings.clone(),
                surface_alpha,
                color_space,
            },
        );
        debug!(handle = %handle, "cached shader assets");
        Ok(ShaderAssets {
            shader_path,
            channel_bindings,
            surface_alpha,
            color_space,
        })
    }
}

fn select_bootstrap_shader(
    config: &MultiConfig,
    cache: &mut ShaderCache<'_>,
) -> Result<ShaderAssets> {
    if let Some(default) = config.default_playlist() {
        if let Some(playlist) = config.playlist(default) {
            if let Some(item) = playlist.items.first() {
                return cache.resolve(&item.handle, false);
            }
        }
    }

    if let Some((_name, playlist)) = config.playlists.iter().next() {
        if let Some(item) = playlist.items.first() {
            return cache.resolve(&item.handle, false);
        }
    }

    bail!("no playlist items available for bootstrap shader")
}

struct PlaylistEngine<'a> {
    config: MultiConfig,
    cache: ShaderCache<'a>,
    scheduler: Scheduler,
    options: EngineOptions,
    refreshed: HashSet<String>,
    targets: HashMap<TargetId, EngineTarget>,
    surface_map: HashMap<SurfaceId, TargetId>,
    workspace_crossfade: Duration,
}

struct EngineTarget {
    surface_id: SurfaceId,
    output_id: Option<OutputId>,
    output_name: Option<String>,
    selector: TargetSelectorKind,
    playlist: String,
    playlist_len: usize,
    crossfade_override: Option<Duration>,
    last_size: Option<(u32, u32)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TargetSelectorKind {
    Workspace(String),
    Output(String),
    Default,
}

impl<'a> PlaylistEngine<'a> {
    fn new(config: MultiConfig, cache: ShaderCache<'a>, seed: u64, options: EngineOptions) -> Self {
        let workspace_crossfade = config.workspace_switch_crossfade();
        let scheduler = Scheduler::new(&config, seed);
        Self {
            config,
            cache,
            scheduler,
            options,
            refreshed: HashSet::new(),
            targets: HashMap::new(),
            surface_map: HashMap::new(),
            workspace_crossfade,
        }
    }

    fn sync_targets(
        &mut self,
        surfaces: &[SurfaceInfo],
        hyprland: Option<&HyprlandSnapshot>,
        now: Instant,
    ) -> Result<Vec<SwapAction>> {
        let mut actions = Vec::new();
        let active: HashSet<SurfaceId> = surfaces.iter().map(|s| s.surface_id).collect();
        let mut to_remove = Vec::new();
        for (surface_id, target_id) in &self.surface_map {
            if !active.contains(surface_id) {
                to_remove.push((*surface_id, target_id.clone()));
            }
        }
        for (surface_id, target_id) in to_remove {
            self.surface_map.remove(&surface_id);
            self.scheduler.remove_target(&target_id);
            self.targets.remove(&target_id);
        }

        for surface in surfaces {
            let target_id = self
                .surface_map
                .entry(surface.surface_id)
                .or_insert_with(|| TargetId::new(format!("surface:{}", surface.surface_id.raw())))
                .clone();
            let resolved = match TargetResolver::new(&self.config).resolve(surface, hyprland) {
                Ok(resolved) => resolved,
                Err(err) => {
                    warn!(
                        surface = surface.surface_id.raw(),
                        output = ?surface.output_name,
                        workspace = hyprland
                            .and_then(|snapshot| surface.output_name.as_ref().and_then(|name| snapshot.workspaces.get(name)))
                            .map(|ws| ws.name.clone()),
                        error = %err,
                        "failed to resolve playlist for surface"
                    );
                    continue;
                }
            };
            trace!(
                surface = surface.surface_id.raw(),
                selector = ?resolved.selector,
                playlist = %resolved.playlist,
                "resolved surface target"
            );
            if let TargetSelectorKind::Workspace(ref key) = resolved.selector {
                trace!(
                    surface = surface.surface_id.raw(),
                    workspace = key,
                    playlist = %resolved.playlist,
                    "workspace selector applied"
                );
            }

            let entry = self.targets.entry(target_id.clone());
            let change = match entry {
                std::collections::hash_map::Entry::Vacant(slot) => {
                    match self
                        .scheduler
                        .set_target(target_id.clone(), &resolved.playlist, now)
                    {
                        Ok(change) => {
                            slot.insert(EngineTarget {
                                surface_id: surface.surface_id,
                                output_id: surface.output_id,
                                output_name: surface.output_name.clone(),
                                selector: resolved.selector.clone(),
                                playlist: resolved.playlist.clone(),
                                playlist_len: resolved.playlist_len,
                                crossfade_override: Some(Duration::ZERO),
                                last_size: surface.size,
                            });
                            info!(
                                target = %target_id.0,
                                selector = ?resolved.selector,
                                playlist = %resolved.playlist,
                                "registered new playlist target"
                            );
                            Some(change)
                        }
                        Err(err) => {
                            warn!(
                                target = %target_id.0,
                                playlist = %resolved.playlist,
                                error = %err,
                                "failed to initialize target playlist"
                            );
                            None
                        }
                    }
                }
                std::collections::hash_map::Entry::Occupied(mut slot) => {
                    let target = slot.get_mut();
                    target.output_id = surface.output_id;
                    target.output_name = surface.output_name.clone();
                    target.last_size = surface.size;
                    if target.selector != resolved.selector || target.playlist != resolved.playlist
                    {
                        let previous_selector = target.selector.clone();
                        let previous_playlist = target.playlist.clone();
                        let was_workspace =
                            matches!(target.selector, TargetSelectorKind::Workspace(_));
                        if was_workspace {
                            trace!(
                                target = %target_id.0,
                                workspace = ?target.selector,
                                "previous workspace selector"
                            );
                        }
                        let change = match self.scheduler.set_target(
                            target_id.clone(),
                            &resolved.playlist,
                            now,
                        ) {
                            Ok(change) => change,
                            Err(err) => {
                                warn!(
                                    target = %target_id.0,
                                    playlist = %resolved.playlist,
                                    error = %err,
                                    "failed to retarget playlist"
                                );
                                continue;
                            }
                        };
                        let is_workspace =
                            matches!(resolved.selector, TargetSelectorKind::Workspace(_));
                        let workspace_switch = was_workspace || is_workspace;
                        let crossfade_override = if workspace_switch {
                            Some(self.workspace_crossfade)
                        } else {
                            None
                        };
                        target.selector = resolved.selector.clone();
                        target.playlist = resolved.playlist.clone();
                        target.playlist_len = resolved.playlist_len;
                        target.crossfade_override = crossfade_override;
                        let crossfade_override_ms = target
                            .crossfade_override
                            .map(|d| d.as_millis())
                            .unwrap_or(0);
                        info!(
                            target = %target_id.0,
                            from_selector = ?previous_selector,
                            to_selector = ?target.selector,
                            from_playlist = %previous_playlist,
                            to_playlist = %target.playlist,
                            workspace_switch,
                            crossfade_override_ms,
                            "retargeted playlist"
                        );
                        Some(change)
                    } else {
                        None
                    }
                }
            };

            if let Some(change) = change {
                if let Some(action) = self.activate_selection(target_id.clone(), change) {
                    actions.push(action);
                }
            }
        }

        Ok(actions)
    }

    fn tick(
        &mut self,
        now: Instant,
        _hyprland: Option<&HyprlandSnapshot>,
    ) -> Result<Vec<SwapAction>> {
        let mut actions = Vec::new();
        for change in self.scheduler.tick(now) {
            let target_id = change.target.clone();
            if let Some(action) = self.activate_selection(target_id, change) {
                actions.push(action);
            }
        }
        Ok(actions)
    }

    fn skip_surface(&mut self, surface_id: SurfaceId, now: Instant) -> Result<Vec<SwapAction>> {
        let Some(target_id) = self.surface_map.get(&surface_id).cloned() else {
            return Ok(Vec::new());
        };
        if let Some(change) = self.scheduler.skip_target(&target_id, now) {
            if let Some(action) = self.activate_selection(target_id, change) {
                return Ok(vec![action]);
            }
        }
        Ok(Vec::new())
    }

    fn activate_selection(
        &mut self,
        target_id: TargetId,
        mut change: scheduler::SelectionChange,
    ) -> Option<SwapAction> {
        let mut attempts = 0;
        loop {
            let handle = change.item.handle.clone();
            let needs_refresh = self.should_refresh(&handle, &change.item);

            let (result, playlist_name, max_attempts) = {
                let target = self.targets.get_mut(&target_id)?;
                let playlist_name = target.playlist.clone();
                let max_attempts = target.playlist_len.max(1);
                let decision = match self.cache.resolve(&handle, needs_refresh) {
                    Ok(assets) => {
                        let crossfade =
                            if let Some(override_duration) = target.crossfade_override.take() {
                                override_duration
                            } else if target.playlist_len <= 1 {
                                Duration::ZERO
                            } else {
                                change.item.crossfade
                            };
                        let display = describe_target(&target_id, target);
                        let selector = SurfaceSelector::Surface(target.surface_id);
                        Ok((assets, crossfade, display, selector))
                    }
                    Err(err) => {
                        warn!(
                            target = %target_id.0,
                            handle = %handle,
                            error = ?err,
                            "failed to load shader; skipping"
                        );
                        Err(())
                    }
                };
                (decision, playlist_name, max_attempts)
            };

            match result {
                Ok((assets, crossfade, display, selector)) => {
                    if needs_refresh {
                        self.refreshed.insert(handle.clone());
                    }

                    debug!(
                        target = %target_id.0,
                        handle = %handle,
                        duration_ms = change.item.duration.as_millis(),
                        crossfade_ms = crossfade.as_millis(),
                        refresh = needs_refresh,
                        "prepared swap action"
                    );

                    let request = build_swap_request(
                        &change.item,
                        &assets,
                        self.options.global_fps,
                        self.options.global_antialias,
                        self.options.global_color_space,
                        crossfade,
                        self.options.prewarm,
                    );
                    return Some(SwapAction {
                        target_display: display,
                        selector,
                        request,
                        handle,
                    });
                }
                Err(()) => {
                    attempts += 1;
                    if attempts >= max_attempts {
                        warn!(
                            target = %target_id.0,
                            playlist = %playlist_name,
                            "all playlist items failed to load"
                        );
                        return None;
                    }

                    if let Some(next) = self.scheduler.skip_target(&target_id, Instant::now()) {
                        change = next;
                        continue;
                    } else {
                        return None;
                    }
                }
            }
        }
    }

    fn should_refresh(&self, handle: &str, item: &ScheduledItem) -> bool {
        if self.options.cache_only {
            return false;
        }
        if !(self.options.refresh_all || item.refresh_once) {
            return false;
        }
        !self.refreshed.contains(handle)
    }
}

fn build_swap_request(
    item: &ScheduledItem,
    assets: &ShaderAssets,
    global_fps: Option<f32>,
    global_antialias: Antialiasing,
    global_color: ColorSpaceMode,
    crossfade: Duration,
    warmup: Duration,
) -> SwapRequest {
    let fps = item.fps.or(global_fps);
    let antialiasing = item
        .antialias
        .map(map_antialias)
        .unwrap_or(global_antialias);
    let color_space = resolve_color_space(global_color, assets.color_space);
    SwapRequest {
        shader_source: assets.shader_path.clone(),
        channel_bindings: assets.channel_bindings.clone(),
        crossfade,
        target_fps: fps,
        antialiasing,
        surface_alpha: assets.surface_alpha,
        color_space,
        warmup,
    }
}

fn map_antialias(setting: AntialiasSetting) -> Antialiasing {
    match setting {
        AntialiasSetting::Auto => Antialiasing::Auto,
        AntialiasSetting::Off => Antialiasing::Off,
        AntialiasSetting::Samples2 => Antialiasing::Samples(2),
        AntialiasSetting::Samples4 => Antialiasing::Samples(4),
        AntialiasSetting::Samples8 => Antialiasing::Samples(8),
        AntialiasSetting::Samples16 => Antialiasing::Samples(16),
    }
}

fn describe_target(target_id: &TargetId, target: &EngineTarget) -> String {
    if let Some(output) = &target.output_name {
        format!("{} ({output})", target_id.0)
    } else {
        target_id.0.clone()
    }
}

struct TargetResolver<'a> {
    config: &'a MultiConfig,
}

struct ResolvedTarget {
    selector: TargetSelectorKind,
    playlist: String,
    playlist_len: usize,
}

impl<'a> TargetResolver<'a> {
    fn new(config: &'a MultiConfig) -> Self {
        Self { config }
    }

    fn resolve(
        &self,
        surface: &SurfaceInfo,
        hyprland: Option<&HyprlandSnapshot>,
    ) -> Result<ResolvedTarget> {
        if let Some(output) = &surface.output_name {
            if let Some(snapshot) = hyprland {
                if let Some(workspace) = snapshot.workspaces.get(output) {
                    if let Some(resolved) = self.resolve_workspace(workspace) {
                        return Ok(resolved);
                    }
                }
            }
            if let Some(resolved) = self.resolve_output(output) {
                return Ok(resolved);
            }
        }
        if let Some(resolved) = self.resolve_default_target() {
            return Ok(resolved);
        }
        bail!(
            "no playlist mapping found for surface {})",
            surface.surface_id.raw()
        )
    }

    fn resolve_workspace(&self, workspace: &WorkspaceInfo) -> Option<ResolvedTarget> {
        let mut candidates = Vec::new();
        candidates.push(format!("workspace:{}", workspace.name));
        candidates.push(format!("workspace:{}", workspace.id));
        candidates.push(workspace.name.clone());
        candidates.push(workspace.id.to_string());
        for key in candidates {
            if let Some(playlist) = self.config.targets.get(&key) {
                return self.build_result(playlist, TargetSelectorKind::Workspace(key));
            }
        }
        None
    }

    fn resolve_output(&self, name: &str) -> Option<ResolvedTarget> {
        let key = format!("output:{name}");
        self.config
            .targets
            .get(&key)
            .and_then(|playlist| self.build_result(playlist, TargetSelectorKind::Output(key)))
    }

    fn resolve_default_target(&self) -> Option<ResolvedTarget> {
        if let Some(playlist) = self.config.targets.get("_default") {
            return self.build_result(playlist, TargetSelectorKind::Default);
        }
        if let Some(default) = self.config.default_playlist() {
            return self.build_result(default, TargetSelectorKind::Default);
        }
        None
    }

    fn build_result(&self, playlist: &str, selector: TargetSelectorKind) -> Option<ResolvedTarget> {
        let playlist_ref = self.config.playlist(playlist)?;
        Some(ResolvedTarget {
            selector,
            playlist: playlist.to_string(),
            playlist_len: playlist_ref.items.len(),
        })
    }
}

struct SwapAction {
    target_display: String,
    selector: SurfaceSelector,
    request: SwapRequest,
    handle: String,
}

struct HyprlandResolver;

impl HyprlandResolver {
    fn new() -> Self {
        Self
    }

    fn snapshot(&mut self) -> Option<HyprlandSnapshot> {
        match HyprlandSnapshot::fetch() {
            Ok(snapshot) => Some(snapshot),
            Err(err) => {
                debug!(error = ?err, "failed to fetch hyprland snapshot");
                None
            }
        }
    }
}

#[derive(Debug, Clone)]
struct HyprlandSnapshot {
    workspaces: HashMap<String, WorkspaceInfo>,
}

#[derive(Debug, Clone, Deserialize)]
struct WorkspaceInfo {
    id: i32,
    name: String,
}

impl HyprlandSnapshot {
    fn fetch() -> Result<Self> {
        let signature = env::var("HYPRLAND_INSTANCE_SIGNATURE")
            .context("HYPRLAND_INSTANCE_SIGNATURE not set")?;

        let mut candidates = Vec::new();
        if let Ok(runtime_dir) = env::var("XDG_RUNTIME_DIR") {
            candidates.push(PathBuf::from(runtime_dir).join("hypr"));
        }
        candidates.push(PathBuf::from("/tmp/hypr"));

        let mut last_error = None;
        let mut stream = None;
        for base in candidates {
            let candidate = base.join(&signature).join(".socket.sock");
            match UnixStream::connect(&candidate) {
                Ok(conn) => {
                    stream = Some(conn);
                    break;
                }
                Err(err) => {
                    last_error = Some((candidate, err));
                }
            }
        }

        let mut stream = match stream {
            Some(stream) => stream,
            None => {
                let (path, err) = last_error
                    .map(|(path, err)| (path, anyhow::Error::new(err)))
                    .unwrap_or_else(|| {
                        (
                            PathBuf::from("/tmp/hypr/.socket.sock"),
                            anyhow::anyhow!("hyprland socket path resolution failed"),
                        )
                    });
                return Err(err.context(format!(
                    "failed to connect to hyprland socket at {}",
                    path.display()
                )));
            }
        };
        stream
            .write_all(b"j/monitors")
            .context("failed to request monitor data")?;
        let _ = stream.shutdown(Shutdown::Write);
        let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
        let mut response = String::new();
        stream
            .read_to_string(&mut response)
            .context("failed to read hyprland response")?;
        let monitors: Vec<MonitorPayload> = serde_json::from_str(response.trim())
            .context("failed to parse hyprland monitor payload")?;
        let mut workspaces = HashMap::new();
        for monitor in monitors {
            if let Some(workspace) = monitor.active_workspace {
                workspaces.insert(monitor.name, workspace);
            }
        }
        Ok(Self { workspaces })
    }
}

#[derive(Debug, Deserialize)]
struct MonitorPayload {
    name: String,
    #[serde(rename = "activeWorkspace")]
    active_workspace: Option<WorkspaceInfo>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::RunArgs;
    use std::path::Path;
    use tempfile::tempdir;

    fn write_pack(root: &Path, name: &str) {
        let pack_dir = root.join(name);
        fs::create_dir_all(&pack_dir).unwrap();
        fs::write(
            pack_dir.join("shader.toml"),
            format!(
                "name = \"{name}\"\nentry = \"image\"\nsurface_alpha = \"opaque\"\n\n[[passes]]\nname = \"image\"\nkind = \"image\"\nsource = \"image.glsl\"\n"
            ),
        )
        .unwrap();
        fs::write(pack_dir.join("image.glsl"), "void main() {}").unwrap();
    }

    fn base_config() -> MultiConfig {
        MultiConfig::from_toml_str(
            r#"
version = 1

[defaults]
playlist = "solo"

[playlists.solo]
mode = "continuous"
item_duration = 60

[[playlists.solo.items]]
handle = "demo"
"#,
        )
        .unwrap()
    }

    #[test]
    fn resolver_prefers_workspace_mapping() {
        let config = MultiConfig::from_toml_str(
            r#"
version = 1

[defaults]
playlist = "ambient"

[playlists.ambient]
mode = "continuous"
item_duration = 60

[[playlists.ambient.items]]
handle = "local/a"

[playlists.focus]
mode = "continuous"
item_duration = 60

[[playlists.focus.items]]
handle = "local/b"

[targets]
"workspace:2" = "focus"
"output:HDMI-A-1" = "ambient"
"_default" = "ambient"
"#,
        )
        .unwrap();

        let resolver = TargetResolver::new(&config);
        let surface = SurfaceInfo {
            surface_id: SurfaceId::from(1u64),
            output_id: Some(OutputId::from(10u64)),
            output_name: Some("HDMI-A-1".to_string()),
            size: None,
        };
        let mut workspaces = HashMap::new();
        workspaces.insert(
            "HDMI-A-1".to_string(),
            WorkspaceInfo {
                id: 2,
                name: "2".to_string(),
            },
        );
        let snapshot = HyprlandSnapshot { workspaces };

        let resolved = resolver
            .resolve(&surface, Some(&snapshot))
            .expect("resolve workspace");
        assert_eq!(resolved.playlist, "focus");
        assert!(matches!(
            resolved.selector,
            TargetSelectorKind::Workspace(_)
        ));
    }

    #[test]
    fn resolver_falls_back_to_output() {
        let config = MultiConfig::from_toml_str(
            r#"
version = 1

[defaults]
playlist = "ambient"

[playlists.ambient]
mode = "continuous"
item_duration = 60

[[playlists.ambient.items]]
handle = "local/a"

[targets]
"output:DP-1" = "ambient"
"#,
        )
        .unwrap();

        let resolver = TargetResolver::new(&config);
        let surface = SurfaceInfo {
            surface_id: SurfaceId::from(1u64),
            output_id: Some(OutputId::from(1u64)),
            output_name: Some("DP-1".to_string()),
            size: None,
        };

        let resolved = resolver
            .resolve(&surface, None)
            .expect("resolve output mapping");
        assert_eq!(resolved.playlist, "ambient");
        assert!(matches!(resolved.selector, TargetSelectorKind::Output(_)));
    }

    #[test]
    fn resolver_defaults_when_no_mapping() {
        let config = base_config();
        let resolver = TargetResolver::new(&config);
        let surface = SurfaceInfo {
            surface_id: SurfaceId::from(1u64),
            output_id: None,
            output_name: None,
            size: None,
        };
        let resolved = resolver
            .resolve(&surface, None)
            .expect("use default playlist");
        assert_eq!(resolved.playlist, "solo");
        assert!(matches!(resolved.selector, TargetSelectorKind::Default));
    }

    #[test]
    fn engine_syncs_new_surface() {
        let temp = tempdir().unwrap();
        let packs_root = temp.path().join("packs");
        let cache_root = temp.path().join("cache");
        fs::create_dir_all(&packs_root).unwrap();

        for name in ["demo", "alt"] {
            let pack_dir = packs_root.join(name);
            fs::create_dir_all(&pack_dir).unwrap();
            fs::write(
                pack_dir.join("shader.toml"),
                format!("name = \"{}\"\nentry = \"image\"\nsurface_alpha = \"opaque\"\n\n[[passes]]\nname = \"image\"\nkind = \"image\"\nsource = \"image.glsl\"\n", name),
            ).unwrap();
            fs::write(pack_dir.join("image.glsl"), "void main() {}").unwrap();
        }

        let repo = ShaderRepository::new(vec![packs_root.clone()], cache_root.clone());
        let cache = ShaderCache::new(&repo, None, false);

        let config = MultiConfig::from_toml_str(
            r#"
version = 1

[defaults]
playlist = "solo"

[playlists.solo]
mode = "continuous"
item_duration = 1

[[playlists.solo.items]]
handle = "demo"

[[playlists.solo.items]]
handle = "alt"
"#,
        )
        .unwrap();
        let options = EngineOptions {
            cache_only: false,
            refresh_all: false,
            global_fps: None,
            global_antialias: Antialiasing::Auto,
            global_color_space: ColorSpaceMode::Auto,
            prewarm: Duration::from_millis(DEFAULT_PREWARM_MS),
        };
        let mut engine = PlaylistEngine::new(config, cache, 99, options);

        let surface = SurfaceInfo {
            surface_id: SurfaceId::from(7u64),
            output_id: None,
            output_name: None,
            size: None,
        };
        let start = Instant::now();
        let actions = engine
            .sync_targets(&[surface], None, start)
            .expect("sync targets");
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].handle, "demo");
        assert_eq!(
            actions[0].request.warmup,
            Duration::from_millis(DEFAULT_PREWARM_MS)
        );

        let advances = engine
            .tick(start + std::time::Duration::from_secs(1), None)
            .expect("tick");
        assert_eq!(advances.len(), 1);
        assert_eq!(advances[0].handle, "alt");
    }

    #[test]
    fn workspace_switch_applies_crossfade_override() {
        let temp = tempdir().unwrap();
        let packs_root = temp.path().join("packs");
        let cache_root = temp.path().join("cache");
        write_pack(&packs_root, "ambient-pack");
        write_pack(&packs_root, "focus-pack");

        let repo = ShaderRepository::new(vec![packs_root.clone()], cache_root);
        let cache = ShaderCache::new(&repo, None, false);

        let config = MultiConfig::from_toml_str(
            r#"
version = 1
workspace_switch_crossfade = "2s"

[defaults]
playlist = "ambient"

[playlists.ambient]
mode = "continuous"
item_duration = 1

[[playlists.ambient.items]]
handle = "ambient-pack"

[playlists.focus]
mode = "continuous"
item_duration = 1

[[playlists.focus.items]]
handle = "focus-pack"

[targets]
"workspace:1" = "focus"
"_default" = "ambient"
"#,
        )
        .unwrap();

        let options = EngineOptions {
            cache_only: false,
            refresh_all: false,
            global_fps: None,
            global_antialias: Antialiasing::Auto,
            global_color_space: ColorSpaceMode::Auto,
            prewarm: Duration::from_millis(DEFAULT_PREWARM_MS),
        };
        let mut engine = PlaylistEngine::new(config, cache, 7, options);

        let surface = SurfaceInfo {
            surface_id: SurfaceId::from(11u64),
            output_id: Some(OutputId::from(5u64)),
            output_name: Some("HDMI-A-1".to_string()),
            size: None,
        };

        let mut workspaces = HashMap::new();
        workspaces.insert(
            "HDMI-A-1".to_string(),
            WorkspaceInfo {
                id: 1,
                name: "1".to_string(),
            },
        );
        let snapshot = HyprlandSnapshot { workspaces };

        let now = Instant::now();
        let initial = engine
            .sync_targets(std::slice::from_ref(&surface), Some(&snapshot), now)
            .expect("initial sync");
        assert_eq!(initial.len(), 1);
        assert_eq!(initial[0].handle, "focus-pack");
        assert_eq!(initial[0].request.crossfade, Duration::ZERO);

        let mut workspaces = HashMap::new();
        workspaces.insert(
            "HDMI-A-1".to_string(),
            WorkspaceInfo {
                id: 3,
                name: "3".to_string(),
            },
        );
        let snapshot = HyprlandSnapshot { workspaces };

        let retarget = engine
            .sync_targets(&[surface], Some(&snapshot), now + Duration::from_secs(1))
            .expect("retarget sync");
        assert_eq!(retarget.len(), 1);
        assert_eq!(retarget[0].handle, "ambient-pack");
        assert_eq!(retarget[0].request.crossfade, Duration::from_secs(2));
    }

    #[test]
    fn engine_skips_missing_items_and_advances() {
        let temp = tempdir().unwrap();
        let packs_root = temp.path().join("packs");
        let cache_root = temp.path().join("cache");
        write_pack(&packs_root, "valid-pack");

        let repo = ShaderRepository::new(vec![packs_root.clone()], cache_root);
        let cache = ShaderCache::new(&repo, None, false);

        let config = MultiConfig::from_toml_str(
            r#"
version = 1

[defaults]
playlist = "solo"

[playlists.solo]
mode = "continuous"
item_duration = 1

[[playlists.solo.items]]
handle = "missing-pack"

[[playlists.solo.items]]
handle = "valid-pack"
"#,
        )
        .unwrap();

        let options = EngineOptions {
            cache_only: false,
            refresh_all: false,
            global_fps: None,
            global_antialias: Antialiasing::Auto,
            global_color_space: ColorSpaceMode::Auto,
            prewarm: Duration::from_millis(DEFAULT_PREWARM_MS),
        };
        let mut engine = PlaylistEngine::new(config, cache, 55, options);

        let surface = SurfaceInfo {
            surface_id: SurfaceId::from(21u64),
            output_id: None,
            output_name: None,
            size: None,
        };
        let actions = engine
            .sync_targets(&[surface], None, Instant::now())
            .expect("sync targets");
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].handle, "valid-pack");
        assert_eq!(actions[0].request.crossfade, Duration::ZERO);
    }

    #[test]
    fn window_mode_requires_default_playlist() {
        let temp = tempdir().unwrap();
        let repo =
            ShaderRepository::new(vec![temp.path().to_path_buf()], temp.path().join("cache"));
        let config = MultiConfig::from_toml_str(
            r#"
version = 1

[playlists.solo]
mode = "continuous"
item_duration = 60

[[playlists.solo.items]]
handle = "demo"
"#,
        )
        .unwrap();

        let args = RunArgs {
            shader: None,
            shadertoy: None,
            multi: None,
            window: true,
            size: None,
            fps: None,
            refresh: false,
            cache_only: false,
            shadertoy_api_key: None,
            antialias: Antialiasing::Auto,
            shader_compiler: Default::default(),
            color_space: ColorSpaceMode::Auto,
            prewarm_ms: None,
            init_defaults: false,
        };

        let err = run_window_multi(&args, &repo, None, config).unwrap_err();
        assert!(err.to_string().contains("defaults.playlist"));
    }
}
