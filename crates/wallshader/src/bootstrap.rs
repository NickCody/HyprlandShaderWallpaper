use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};
use renderer::RendererConfig;
use shadertoy::normalize_shadertoy_reference;
use tracing::debug;

use crate::cli::RunArgs;
use crate::handles::{EntryHandle, LaunchHandle};
use crate::paths::AppPaths;
pub fn bootstrap_filesystem(paths: &AppPaths) -> Result<()> {
    let directories = vec![
        paths.config_dir().to_path_buf(),
        paths.data_dir().to_path_buf(),
        paths.cache_dir().to_path_buf(),
        paths.cache_dir().join("shadertoy"),
    ];

    for dir in directories {
        ensure_directory(&dir)?;
    }

    Ok(())
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
            format!(
                "failed to create wallshader directory at {}",
                path.display()
            )
        })?;
        debug!(path = %path.display(), "created wallshader directory");
        Ok(())
    }
}

pub fn resolve_entry_handle(args: &RunArgs) -> Result<EntryHandle> {
    if let Some(ref shadertoy) = args.shadertoy {
        let normalized = normalize_shadertoy_reference(shadertoy)?;
        let handle = EntryHandle::parse(&normalized)?;
        return Ok(handle);
    }

    if let Some(launch) = args.shader.as_ref() {
        match launch.inner() {
            LaunchHandle::Entry(entry) => return Ok(entry.clone()),
            LaunchHandle::Playlist(_) => {
                bail!("playlist handle is not valid when resolving an entry shader")
            }
        }
    }

    Ok(EntryHandle::LocalPack {
        name: "local-demo".to_string(),
    })
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
