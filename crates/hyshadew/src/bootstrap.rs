use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};
use renderer::RendererConfig;
use shadertoy::ShaderHandle;
use tracing::{debug, info};

use crate::cli::Args;
use crate::paths::AppPaths;
use crate::state::AppState;

pub fn bootstrap_filesystem(paths: &AppPaths) -> Result<AppState> {
    let directories = vec![
        paths.config_dir().to_path_buf(),
        paths.data_dir().to_path_buf(),
        paths.cache_dir().to_path_buf(),
        paths.config_dir().join("local-shaders"),
        paths.config_dir().join("multi"),
        paths.data_dir().join("local-shaders"),
        paths.data_dir().join("multi"),
        paths.cache_dir().join("shadertoy"),
    ];

    for dir in directories {
        ensure_directory(&dir)?;
    }

    let state_path = paths.state_file();
    let state_exists = state_path.exists();
    let state = AppState::load_or_default(&state_path)?;
    if state_exists {
        debug!(path = %state_path.display(), "loaded hyshadew state file");
    } else {
        state.persist(&state_path)?;
        info!(path = %state_path.display(), "initialised hyshadew state file");
    }

    Ok(state)
}

fn ensure_directory(path: &Path) -> Result<()> {
    if path.exists() {
        if path.is_dir() {
            debug!(path = %path.display(), "reusing existing directory");
            Ok(())
        } else {
            bail!("filesystem entry at {} is not a directory", path.display());
        }
    } else {
        fs::create_dir_all(path).with_context(|| {
            format!("failed to create hyshadew directory at {}", path.display())
        })?;
        info!(path = %path.display(), "created hyshadew directory");
        Ok(())
    }
}

pub fn resolve_shader_handle(args: &Args) -> Result<ShaderHandle> {
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

pub fn normalize_shadertoy_reference(input: &str) -> Result<String> {
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
            .map(|segment| segment.split(&['?', '#'][..]).next().unwrap_or(segment))
            .filter(|id| !id.is_empty());

        if let Some(id) = id_candidate {
            return Ok(format!("shadertoy://{}", id));
        } else {
            bail!("unable to extract shader id from shadertoy url");
        }
    }

    Ok(format!("shadertoy://{}", trimmed))
}

pub fn parse_surface_size(spec: &str) -> Result<(u32, u32)> {
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

pub struct SingleRunConfig {
    pub renderer_config: RendererConfig,
}
