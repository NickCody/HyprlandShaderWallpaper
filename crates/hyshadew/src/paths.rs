use std::env;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use directories_next::ProjectDirs;

pub const ENV_CONFIG_DIR: &str = "HYSHADEW_CONFIG_DIR";
pub const ENV_DATA_DIR: &str = "HYSHADEW_DATA_DIR";
pub const ENV_CACHE_DIR: &str = "HYSHADEW_CACHE_DIR";
pub const ENV_SHARE_DIR: &str = "HYSHADEW_SHARE_DIR";

const QUALIFIER: &str = "org";
const ORGANISATION: &str = "Hyprland";
const APPLICATION: &str = "Hyshadew";

#[derive(Debug, Clone)]
pub struct AppPaths {
    config_dir: PathBuf,
    data_dir: PathBuf,
    cache_dir: PathBuf,
    share_dir: PathBuf,
}

impl AppPaths {
    pub fn discover() -> Result<Self> {
        let project_dirs = ProjectDirs::from(QUALIFIER, ORGANISATION, APPLICATION)
            .ok_or_else(|| anyhow!("failed to determine user directories"))?;

        let config_dir = resolve_dir(ENV_CONFIG_DIR, project_dirs.config_dir())
            .context("failed to resolve hyshadew config directory")?;
        let data_dir = resolve_dir(ENV_DATA_DIR, project_dirs.data_dir())
            .context("failed to resolve hyshadew data directory")?;
        let cache_dir = resolve_dir(ENV_CACHE_DIR, project_dirs.cache_dir())
            .context("failed to resolve hyshadew cache directory")?;
        let share_dir = resolve_share_dir(&project_dirs)?;

        Ok(Self {
            config_dir,
            data_dir,
            cache_dir,
            share_dir,
        })
    }

    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    pub fn share_dir(&self) -> &Path {
        &self.share_dir
    }

    pub fn state_file(&self) -> PathBuf {
        self.config_dir.join("state.toml")
    }

    pub fn shader_user_dirs(&self) -> Vec<PathBuf> {
        vec![
            self.config_dir.join("local-shaders"),
            self.data_dir.join("local-shaders"),
        ]
    }

    pub fn shader_roots(&self) -> Vec<PathBuf> {
        let mut roots = self.shader_user_dirs();
        roots.push(self.share_dir.join("local-shaders"));
        roots
    }

    pub fn playlist_user_dirs(&self) -> Vec<PathBuf> {
        vec![self.config_dir.join("multi"), self.data_dir.join("multi")]
    }

    pub fn playlist_roots(&self) -> Vec<PathBuf> {
        let mut roots = self.playlist_user_dirs();
        roots.push(self.share_dir.join("multi"));
        roots
    }

    pub fn shadertoy_cache_dir(&self) -> PathBuf {
        self.cache_dir.join("shadertoy")
    }
}

fn resolve_dir(env_var: &str, default: &Path) -> Result<PathBuf> {
    if let Some(value) = env_override(env_var) {
        return Ok(value);
    }
    Ok(default.to_path_buf())
}

fn resolve_share_dir(project_dirs: &ProjectDirs) -> Result<PathBuf> {
    if let Some(value) = env_override(ENV_SHARE_DIR) {
        return Ok(value);
    }

    Ok(default_share_dir(project_dirs))
}

fn env_override(name: &str) -> Option<PathBuf> {
    match env::var_os(name) {
        Some(value) if !value.as_os_str().is_empty() => Some(PathBuf::from(value)),
        _ => None,
    }
}

#[cfg(target_family = "unix")]
fn default_share_dir(_: &ProjectDirs) -> PathBuf {
    PathBuf::from("/usr/share/hyshadew")
}

#[cfg(not(target_family = "unix"))]
fn default_share_dir(project_dirs: &ProjectDirs) -> PathBuf {
    project_dirs.data_dir().to_path_buf()
}
