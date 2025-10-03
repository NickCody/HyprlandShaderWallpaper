use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use tracing::{debug, info, warn};

use crate::paths::AppPaths;
use crate::state::AppState;

#[derive(Debug, Default)]
pub struct DefaultsSyncReport {
    pub copied_assets: Vec<DefaultCopy>,
    pub share_version: Option<String>,
}

impl DefaultsSyncReport {
    pub fn copied_any(&self) -> bool {
        !self.copied_assets.is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct DefaultCopy {
    pub source: PathBuf,
    pub target: PathBuf,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SyncOptions {
    pub dry_run: bool,
}

pub fn sync_defaults(
    paths: &AppPaths,
    state: &mut AppState,
    options: SyncOptions,
) -> Result<DefaultsSyncReport> {
    let share_dir = paths.share_dir();
    if !share_dir.exists() {
        debug!(path = %share_dir.display(), "no system defaults directory present; skipping sync");
        return Ok(DefaultsSyncReport::default());
    }

    let share_version = read_share_version(share_dir).transpose()?;

    let mut report = DefaultsSyncReport {
        copied_assets: vec![],
        share_version,
    };

    report.copied_assets = copy_missing_children(
        &share_dir.join("local-shaders"),
        &paths.data_dir().join("local-shaders"),
        options.dry_run,
    )?;

    if let Some(version) = &report.share_version {
        if state.defaults_version.as_deref() != Some(version.as_str()) {
            info!(
                current = state.defaults_version.as_deref(),
                available = %version,
                "system defaults version updated"
            );
        }
    }

    if !options.dry_run {
        if report.copied_any() {
            info!(
                assets = report.copied_assets.len(),
                "synced bundled defaults into user directory"
            );
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        state.last_defaults_sync = Some(now.to_string());

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

fn copy_missing_children(
    source_root: &Path,
    target_root: &Path,
    dry_run: bool,
) -> Result<Vec<DefaultCopy>> {
    if !source_root.exists() {
        debug!(path = %source_root.display(), "no defaults present at source path");
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

        if !dry_run {
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
        } else if !file_type.is_dir() && !file_type.is_file() {
            warn!(path = %source_path.display(), "skipping unsupported default entry type");
            continue;
        }

        copied.push(DefaultCopy {
            source: source_path.clone(),
            target: target_path.clone(),
        });

        if dry_run {
            info!(
                source = %source_path.display(),
                target = %target_path.display(),
                "bundled default would be installed (dry-run)"
            );
        } else {
            info!(
                source = %source_path.display(),
                target = %target_path.display(),
                "installed bundled default"
            );
        }
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

#[derive(Debug, Clone)]
pub struct DefaultEntryStatus {
    pub name: String,
    pub source: PathBuf,
    pub target: PathBuf,
    pub installed: bool,
}

pub fn enumerate_defaults(paths: &AppPaths) -> Result<Vec<DefaultEntryStatus>> {
    let share_dir = paths.share_dir();
    if !share_dir.exists() {
        debug!(path = %share_dir.display(), "no system defaults directory present; skipping enumeration");
        return Ok(Vec::new());
    }

    let share_root = share_dir.join("local-shaders");
    if !share_root.exists() {
        return Ok(Vec::new());
    }

    let target_root = paths.data_dir().join("local-shaders");

    let mut statuses = Vec::new();
    for entry in fs::read_dir(&share_root)
        .with_context(|| format!("failed to list defaults in {}", share_root.display()))?
    {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        let source_path = entry.path();
        let target_path = target_root.join(&name);
        statuses.push(DefaultEntryStatus {
            name,
            source: source_path,
            target: target_path.clone(),
            installed: target_path.exists(),
        });
    }

    statuses.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(statuses)
}

#[derive(Debug, Clone)]
pub struct PathOverview {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub share_dir: PathBuf,
    pub shader_roots: Vec<PathBuf>,
    pub shadertoy_cache: PathBuf,
    pub state_file: PathBuf,
}

pub fn describe_paths(paths: &AppPaths) -> PathOverview {
    PathOverview {
        config_dir: paths.config_dir().to_path_buf(),
        data_dir: paths.data_dir().to_path_buf(),
        cache_dir: paths.cache_dir().to_path_buf(),
        share_dir: paths.share_dir().to_path_buf(),
        shader_roots: paths.shader_roots(),
        shadertoy_cache: paths.shadertoy_cache_dir(),
        state_file: paths.state_file(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_share_layout(root: &Path) {
        let shaders_root = root.join("local-shaders");
        fs::create_dir_all(shaders_root.join("demo")).unwrap();

        fs::write(shaders_root.join("demo/shader.toml"), "name = \"Demo\"").unwrap();
        fs::write(
            shaders_root.join("demo-playlist.toml"),
            "playlist = \"demo\"",
        )
        .unwrap();
        fs::write(root.join("VERSION"), "1.0.0\n").unwrap();
    }

    fn build_paths(root: &Path) -> AppPaths {
        let config = root.join("config");
        let data = root.join("data");
        let cache = root.join("cache");
        let share = root.join("share");
        fs::create_dir_all(&config).unwrap();
        fs::create_dir_all(&data).unwrap();
        fs::create_dir_all(&cache).unwrap();
        fs::create_dir_all(&share).unwrap();

        AppPaths::from_raw(config, data, cache, share)
    }

    #[test]
    fn sync_defaults_copies_missing_assets() {
        let root = TempDir::new().unwrap();
        let paths = build_paths(root.path());

        create_share_layout(&paths.share_dir());

        let mut state = AppState::default();
        let report = sync_defaults(&paths, &mut state, SyncOptions::default()).unwrap();

        assert_eq!(report.copied_assets.len(), 2);
        assert_eq!(report.share_version.as_deref(), Some("1.0.0"));

        let shader_target = paths.data_dir().join("local-shaders/demo/shader.toml");
        let playlist_target = paths.data_dir().join("local-shaders/demo-playlist.toml");

        assert!(shader_target.exists());
        assert!(playlist_target.exists());
        assert!(state.last_defaults_sync.is_some());
        assert_eq!(state.defaults_version.as_deref(), Some("1.0.0"));

        let shader_contents = fs::read_to_string(shader_target).unwrap();
        assert_eq!(shader_contents, "name = \"Demo\"");
    }

    #[test]
    fn sync_defaults_is_idempotent() {
        let root = TempDir::new().unwrap();
        let paths = build_paths(root.path());

        create_share_layout(&paths.share_dir());

        let mut state = AppState::default();
        sync_defaults(&paths, &mut state, SyncOptions::default()).unwrap();

        let report = sync_defaults(&paths, &mut state, SyncOptions::default()).unwrap();

        assert!(report.copied_assets.is_empty());
        assert_eq!(state.defaults_version.as_deref(), Some("1.0.0"));
    }

    #[test]
    fn sync_defaults_preserves_user_modifications() {
        let root = TempDir::new().unwrap();
        let paths = build_paths(root.path());

        create_share_layout(&paths.share_dir());

        let mut state = AppState::default();
        sync_defaults(&paths, &mut state, SyncOptions::default()).unwrap();

        let shader_target = paths.data_dir().join("local-shaders/demo/shader.toml");
        fs::write(&shader_target, "name = \"User\"").unwrap();

        let report = sync_defaults(&paths, &mut state, SyncOptions::default()).unwrap();

        assert!(report.copied_assets.is_empty());

        let shader_contents = fs::read_to_string(shader_target).unwrap();
        assert_eq!(shader_contents, "name = \"User\"");
    }

    #[test]
    fn sync_defaults_dry_run_reports_without_copying() {
        let root = TempDir::new().unwrap();
        let paths = build_paths(root.path());

        create_share_layout(&paths.share_dir());

        let mut state = AppState::default();
        let report = sync_defaults(&paths, &mut state, SyncOptions { dry_run: true }).unwrap();

        assert_eq!(report.copied_assets.len(), 2);
        assert!(!paths
            .data_dir()
            .join("local-shaders/demo/shader.toml")
            .exists());
        assert!(!paths
            .data_dir()
            .join("local-shaders/demo-playlist.toml")
            .exists());
        assert!(state.last_defaults_sync.is_none());
        assert!(state.defaults_version.is_none());
    }
}
