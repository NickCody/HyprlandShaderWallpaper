//! Prepares the daemon for launch by creating directories, unpacking bundled assets, and
//! translating CLI entry points into renderer-friendly shapes that `run.rs` and `multi.rs`
//! expect, bridging user input with low-level rendering configuration.
//!
//! Types:
//!
//! - `SingleRunConfig` wraps the renderer bootstrap payload for single-shader mode.
//!
//! Functions:
//!
//! - `bootstrap_filesystem` and `extract_bundled_assets` ready the on-disk layout.
//! - `resolve_entry_handle` and `parse_surface_size` interpret CLI arguments.
//! - `ensure_directory` and `copy_recursively` implement AppImage-flavoured asset updates.

use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};
use renderer::RendererConfig;
use shadertoy::normalize_shadertoy_reference;
use tracing::{debug, info};

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

    // First-run AppImage asset extraction: copy bundled shaders and playlists from
    // SHARE_DIR to DATA_DIR, always overwriting. This ensures AppImage users get updates.
    extract_bundled_assets(paths)?;

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
        fs::create_dir_all(path)
            .with_context(|| format!("failed to create wax11 directory at {}", path.display()))?;
        debug!(path = %path.display(), "created wax11 directory");
        Ok(())
    }
}

/// Extract bundled assets from SHARE_DIR to DATA_DIR (for AppImage first-run setup).
/// Always overwrites existing assets in DATA_DIR so users get updates.
/// Users should customize in CONFIG_DIR instead.
fn extract_bundled_assets(paths: &AppPaths) -> Result<()> {
    let share_dir = paths.share_dir();
    let data_dir = paths.data_dir();

    // Skip if share_dir doesn't exist or is the same as data_dir
    if !share_dir.exists() || share_dir == data_dir {
        return Ok(());
    }

    let marker_file = data_dir.join(".bundled-assets-version");
    let current_version = env!("CARGO_PKG_VERSION");

    // Check if we've already extracted this version
    let needs_extraction = if marker_file.exists() {
        match fs::read_to_string(&marker_file) {
            Ok(version) => version.trim() != current_version,
            Err(_) => true,
        }
    } else {
        true
    };

    if !needs_extraction {
        debug!(
            share = %share_dir.display(),
            data = %data_dir.display(),
            version = current_version,
            "bundled assets already extracted for this version"
        );
        return Ok(());
    }

    info!(
        share = %share_dir.display(),
        data = %data_dir.display(),
        version = current_version,
        "extracting bundled shader assets to data directory"
    );

    // Extract shader packs
    let share_shaders = share_dir.join("shaders");
    if share_shaders.is_dir() {
        let data_shaders = data_dir.join("shaders");
        fs::create_dir_all(&data_shaders)?;

        for entry in fs::read_dir(&share_shaders)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                let name = entry.file_name();
                let dest = data_shaders.join(&name);

                // Remove existing pack and copy fresh
                if dest.exists() {
                    fs::remove_dir_all(&dest)?;
                }
                copy_recursively(&path, &dest)?;
                debug!(pack = ?name, "extracted shader pack");
            }
        }
    }

    // Extract playlists
    let share_playlists = share_dir.join("playlists");
    if share_playlists.is_dir() {
        let data_playlists = data_dir.join("playlists");
        fs::create_dir_all(&data_playlists)?;

        for entry in fs::read_dir(&share_playlists)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("toml") {
                let name = entry.file_name();
                let dest = data_playlists.join(&name);
                fs::copy(&path, &dest)?;
                debug!(playlist = ?name, "extracted playlist");
            }
        }
    }

    // Write marker file
    fs::write(&marker_file, current_version)?;
    info!("bundled asset extraction complete");

    Ok(())
}

fn copy_recursively(src: &Path, dst: &Path) -> Result<()> {
    if src.is_dir() {
        fs::create_dir_all(dst)?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            let target = dst.join(entry.file_name());
            if file_type.is_dir() {
                copy_recursively(&entry.path(), &target)?;
            } else if file_type.is_symlink() {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::symlink;
                    let link = fs::read_link(entry.path())?;
                    symlink(link, &target)?;
                }
                #[cfg(not(unix))]
                {
                    fs::copy(entry.path(), &target)?;
                }
            } else {
                fs::copy(entry.path(), &target)?;
            }
        }
    } else {
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(src, dst)?;
    }

    Ok(())
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
