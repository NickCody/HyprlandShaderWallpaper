use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use tracing::{debug, info, warn};

use crate::paths::AppPaths;
use crate::state::AppState;

#[derive(Debug, Default)]
pub struct DefaultsSyncReport {
    pub copied_shader_packs: Vec<PathBuf>,
    pub copied_playlists: Vec<PathBuf>,
    pub share_version: Option<String>,
}

impl DefaultsSyncReport {
    pub fn copied_any(&self) -> bool {
        !(self.copied_shader_packs.is_empty() && self.copied_playlists.is_empty())
    }
}

pub fn sync_defaults(paths: &AppPaths, state: &mut AppState) -> Result<DefaultsSyncReport> {
    let share_dir = paths.share_dir();
    if !share_dir.exists() {
        debug!(path = %share_dir.display(), "no system defaults directory present; skipping sync");
        return Ok(DefaultsSyncReport::default());
    }

    let share_version = read_share_version(share_dir).transpose()?;

    let mut report = DefaultsSyncReport {
        copied_shader_packs: vec![],
        copied_playlists: vec![],
        share_version,
    };

    report.copied_shader_packs = copy_missing_children(
        &share_dir.join("local-shaders"),
        &paths.data_dir().join("local-shaders"),
    )?;

    report.copied_playlists =
        copy_missing_children(&share_dir.join("multi"), &paths.data_dir().join("multi"))?;

    if let Some(version) = &report.share_version {
        if state.defaults_version.as_deref() != Some(version.as_str()) {
            info!(
                current = state.defaults_version.as_deref(),
                available = %version,
                "system defaults version updated"
            );
        }
    }

    if report.copied_any() {
        info!(
            shaders = report.copied_shader_packs.len(),
            playlists = report.copied_playlists.len(),
            "synced bundled defaults into user directory"
        );
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        state.last_defaults_sync = Some(now.to_string());
    }

    if report.copied_any() {
        if let Some(version) = &report.share_version {
            state.defaults_version = Some(version.clone());
        }
    }

    Ok(report)
}

fn read_share_version(share_dir: &Path) -> Option<Result<String>> {
    let version_path = share_dir.join("VERSION");
    if !version_path.exists() {
        return None;
    }

    Some(
        fs::read_to_string(&version_path)
            .map(|value| value.trim().to_string())
            .map_err(|err| anyhow::anyhow!("failed to read defaults VERSION file: {err}")),
    )
}

fn copy_missing_children(source_root: &Path, target_root: &Path) -> Result<Vec<PathBuf>> {
    if !source_root.exists() {
        debug!(path = %source_root.display(), "no defaults present for category");
        return Ok(Vec::new());
    }

    let mut copied = Vec::new();
    for entry in fs::read_dir(source_root)
        .with_context(|| format!("failed to list defaults in {}", source_root.display()))?
    {
        let entry = entry?;
        let name = entry.file_name();
        let source_path = entry.path();
        let target_path = target_root.join(&name);

        if target_path.exists() {
            debug!(
                source = %source_path.display(),
                target = %target_path.display(),
                "skipping default asset; already present"
            );
            continue;
        }

        let file_type = entry.file_type()?;

        if file_type.is_dir() {
            copy_dir_recursive(&source_path, &target_path).with_context(|| {
                format!(
                    "failed to copy default directory {} -> {}",
                    source_path.display(),
                    target_path.display()
                )
            })?;
        } else if file_type.is_file() {
            if let Some(parent) = target_path.parent() {
                fs::create_dir_all(parent).with_context(|| {
                    format!(
                        "failed to prepare directory for default file {}",
                        parent.display()
                    )
                })?;
            }
            fs::copy(&source_path, &target_path).with_context(|| {
                format!(
                    "failed to copy default file {} -> {}",
                    source_path.display(),
                    target_path.display()
                )
            })?;
        } else {
            warn!(path = %source_path.display(), "skipping unsupported default entry type");
            continue;
        }

        copied.push(target_path.clone());
        info!(
            source = %source_path.display(),
            target = %target_path.display(),
            "installed bundled default"
        );
    }

    Ok(copied)
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst).with_context(|| {
        format!(
            "failed to create directory {} while copying defaults",
            dst.display()
        )
    })?;

    for entry in fs::read_dir(src)
        .with_context(|| format!("failed to enumerate default directory {}", src.display()))?
    {
        let entry = entry?;
        let entry_type = entry.file_type()?;
        let entry_path = entry.path();
        let target_path = dst.join(entry.file_name());

        if entry_type.is_dir() {
            copy_dir_recursive(&entry_path, &target_path)?;
        } else if entry_type.is_file() {
            fs::copy(&entry_path, &target_path).with_context(|| {
                format!(
                    "failed to copy default file {} -> {}",
                    entry_path.display(),
                    target_path.display()
                )
            })?;
        } else {
            warn!(path = %entry_path.display(), "skipping unsupported default entry type");
        }
    }

    Ok(())
}
