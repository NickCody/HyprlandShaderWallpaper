//! Discovers and normalises wax11 shader's config/data/cache/share directory roots,
//! handling environment variable overrides, developer mode, and installer layouts
//! so CLI tooling and runtime modules resolve shader packs consistently.
//!
//! Types:
//!
//! - `AppPaths` captures resolved directories and exposes shader/playlist search orders.
//!
//! Functions:
//!
//! - `AppPaths::discover` probes environment variables and defaults to establish directory structure.
//! - Accessors like `shader_roots` and `playlist_roots` encode resolution precedence.
//! - Internal helpers manage environment expansion and developer root detection.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use directories_next::ProjectDirs;

pub const ENV_CONFIG_DIR: &str = "WAX11_CONFIG_DIR";
pub const ENV_DATA_DIR: &str = "WAX11_DATA_DIR";
pub const ENV_CACHE_DIR: &str = "WAX11_CACHE_DIR";
pub const ENV_SHARE_DIR: &str = "WAX11_SHARE_DIR";

const QUALIFIER: &str = "org";
const ORGANISATION: &str = "wax11";
const APPLICATION: &str = "wax11";
const ENV_DEV_ROOT: &str = "WAX11_DEV_ROOT";

#[derive(Debug, Clone)]
pub struct AppPaths {
    config_dir: PathBuf,
    data_dir: PathBuf,
    cache_dir: PathBuf,
    share_dir: PathBuf,
    dev_root: Option<PathBuf>,
}

impl AppPaths {
    pub fn discover() -> Result<Self> {
        let project_dirs = ProjectDirs::from(QUALIFIER, ORGANISATION, APPLICATION)
            .ok_or_else(|| anyhow!("failed to determine user directories"))?;

        let config_dir = resolve_directory(ENV_CONFIG_DIR, project_dirs.config_dir(), "config")?;
        let data_dir = resolve_directory(ENV_DATA_DIR, project_dirs.data_dir(), "data")?;
        let cache_dir = resolve_directory(ENV_CACHE_DIR, project_dirs.cache_dir(), "cache")?;

        let share_dir = resolve_share_dir(&project_dirs)?;
        let dev_root = detect_dev_root();

        Ok(Self {
            config_dir,
            data_dir,
            cache_dir,
            share_dir,
            dev_root,
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
        // Resolution order: CONFIG_DIR > DATA_DIR > SHARE_DIR
        // Users should copy shaders to CONFIG_DIR for customization.
        // DATA_DIR is populated by installers and AppImage extraction.
        vec![
            self.config_dir.join("shaders"),
            self.data_dir.join("shaders"),
            self.share_dir.join("shaders"),
        ]
    }

    pub fn shader_roots(&self) -> Vec<PathBuf> {
        // Resolution order: CONFIG_DIR > DATA_DIR > SHARE_DIR > dev_root
        // This ensures user customizations in CONFIG_DIR take priority over
        // installed assets in DATA_DIR and bundled assets in SHARE_DIR.
        let mut roots = vec![self.config_dir.join("shaders")];
        roots.extend(self.shader_user_dirs().into_iter().skip(1)); // Skip config_dir/shaders (already added)
        roots.push(self.share_dir.join("shaders"));
        if let Some(dev_root) = &self.dev_root {
            let candidate = dev_root.join("shaders");
            if !roots.contains(&candidate) {
                roots.push(candidate);
            }
        }
        roots
    }

    pub fn shadertoy_cache_dir(&self) -> PathBuf {
        self.cache_dir.join("shadertoy")
    }

    pub fn playlist_user_dirs(&self) -> Vec<PathBuf> {
        // Resolution order: CONFIG_DIR > DATA_DIR
        // Users should copy playlists to CONFIG_DIR for customization.
        vec![
            self.config_dir.join("playlists"),
            self.data_dir.join("playlists"),
        ]
    }

    pub fn playlist_roots(&self) -> Vec<PathBuf> {
        // Resolution order: CONFIG_DIR > DATA_DIR > SHARE_DIR > dev_root
        // This ensures user customizations in CONFIG_DIR take priority.
        let mut roots = self.playlist_user_dirs();
        roots.push(self.share_dir.join("playlists"));
        if let Some(dev_root) = &self.dev_root {
            let candidate = dev_root.join("playlists");
            if !roots.contains(&candidate) {
                roots.push(candidate);
            }
        }
        roots
    }
}

fn resolve_directory(primary_env: &str, primary_default: &Path, label: &str) -> Result<PathBuf> {
    if let Some(value) = env_override(primary_env) {
        return Ok(value);
    }

    let primary = primary_default.to_path_buf();

    // Create directory if it doesn't exist
    if !primary.exists() {
        fs::create_dir_all(&primary).with_context(|| {
            format!(
                "failed to create wax11 {label} directory at {}",
                primary.display()
            )
        })?;
    }

    Ok(primary)
}

fn resolve_share_dir(project_dirs: &ProjectDirs) -> Result<PathBuf> {
    if let Some(value) = env_override(ENV_SHARE_DIR) {
        return Ok(value);
    }

    let primary = default_share_dir(project_dirs);
    Ok(primary)
}

fn env_override(name: &str) -> Option<PathBuf> {
    match env::var_os(name) {
        Some(value) if !value.as_os_str().is_empty() => Some(PathBuf::from(value)),
        _ => None,
    }
}

fn detect_dev_root() -> Option<PathBuf> {
    if let Some(explicit) = env_override(ENV_DEV_ROOT) {
        return Some(explicit);
    }

    let mut current = env::current_dir().ok()?;
    loop {
        let candidate = current.join("shaders");
        if candidate.is_dir() {
            return Some(current.clone());
        }
        match current.parent() {
            Some(parent) => current = parent.to_path_buf(),
            None => break,
        }
    }
    None
}

#[cfg(target_family = "unix")]
fn default_share_dir(_: &ProjectDirs) -> PathBuf {
    PathBuf::from("/usr/share/wax11")
}

#[cfg(not(target_family = "unix"))]
fn default_share_dir(project_dirs: &ProjectDirs) -> PathBuf {
    project_dirs.data_dir().to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::path::Path;
    use std::sync::{Mutex, OnceLock};
    use tempfile::TempDir;

    fn env_lock() -> &'static Mutex<()> {
        static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        ENV_LOCK.get_or_init(|| Mutex::new(()))
    }

    struct EnvGuard {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &Path) -> Self {
            let previous = env::var_os(key);
            env::set_var(key, value);
            Self { key, previous }
        }

        fn clear(key: &'static str) -> Self {
            let previous = env::var_os(key);
            env::remove_var(key);
            Self { key, previous }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(value) = self.previous.take() {
                env::set_var(self.key, value);
            } else {
                env::remove_var(self.key);
            }
        }
    }

    #[test]
    fn env_overrides_take_precedence() {
        let _guard = env_lock().lock().unwrap();
        let root = TempDir::new().unwrap();
        let config_dir = root.path().join("config");
        let data_dir = root.path().join("data");
        let cache_dir = root.path().join("cache");
        let share_dir = root.path().join("share");

        let _config_guard = EnvGuard::set(ENV_CONFIG_DIR, &config_dir);
        let _data_guard = EnvGuard::set(ENV_DATA_DIR, &data_dir);
        let _cache_guard = EnvGuard::set(ENV_CACHE_DIR, &cache_dir);
        let _share_guard = EnvGuard::set(ENV_SHARE_DIR, &share_dir);

        let paths = AppPaths::discover().unwrap();

        assert_eq!(paths.config_dir(), config_dir.as_path());
        assert_eq!(paths.data_dir(), data_dir.as_path());
        assert_eq!(paths.cache_dir(), cache_dir.as_path());
        assert_eq!(paths.share_dir(), share_dir.as_path());
    }

    #[cfg(target_family = "unix")]
    #[test]
    fn default_share_dir_on_unix_matches_usr_share() {
        let _guard = env_lock().lock().unwrap();
        let root = TempDir::new().unwrap();
        let config_dir = root.path().join("config");
        let data_dir = root.path().join("data");
        let cache_dir = root.path().join("cache");

        let _config_guard = EnvGuard::set(ENV_CONFIG_DIR, &config_dir);
        let _data_guard = EnvGuard::set(ENV_DATA_DIR, &data_dir);
        let _cache_guard = EnvGuard::set(ENV_CACHE_DIR, &cache_dir);
        let _share_guard = EnvGuard::clear(ENV_SHARE_DIR);

        let paths = AppPaths::discover().unwrap();

        assert_eq!(paths.share_dir(), Path::new("/usr/share/wax11"));
    }
}
