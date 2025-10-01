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

#[cfg(test)]
impl AppPaths {
    pub fn from_raw(
        config_dir: PathBuf,
        data_dir: PathBuf,
        cache_dir: PathBuf,
        share_dir: PathBuf,
    ) -> Self {
        Self {
            config_dir,
            data_dir,
            cache_dir,
            share_dir,
        }
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

        assert_eq!(paths.share_dir(), Path::new("/usr/share/hyshadew"));
    }
}
