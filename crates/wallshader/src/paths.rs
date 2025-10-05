use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use directories_next::ProjectDirs;
use tracing::{debug, warn};

pub const ENV_CONFIG_DIR: &str = "WALLSHADER_CONFIG_DIR";
pub const ENV_DATA_DIR: &str = "WALLSHADER_DATA_DIR";
pub const ENV_CACHE_DIR: &str = "WALLSHADER_CACHE_DIR";
pub const ENV_SHARE_DIR: &str = "WALLSHADER_SHARE_DIR";

const QUALIFIER: &str = "org";
const ORGANISATION: &str = "WallShaderade";
const APPLICATION: &str = "wallshader";

const LEGACY_ENV_CONFIG_DIR: &str = "HYSHADEW_CONFIG_DIR";
const LEGACY_ENV_DATA_DIR: &str = "HYSHADEW_DATA_DIR";
const LEGACY_ENV_CACHE_DIR: &str = "HYSHADEW_CACHE_DIR";
const LEGACY_ENV_SHARE_DIR: &str = "HYSHADEW_SHARE_DIR";

const LEGACY_ORGANISATION: &str = "Hyprland";
const LEGACY_APPLICATION: &str = "Hyshadew";
const ENV_DEV_ROOT: &str = "WALLSHADER_DEV_ROOT";

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
        let legacy_dirs = ProjectDirs::from(QUALIFIER, LEGACY_ORGANISATION, LEGACY_APPLICATION);

        let config_dir = resolve_directory(
            ENV_CONFIG_DIR,
            project_dirs.config_dir(),
            LEGACY_ENV_CONFIG_DIR,
            legacy_dirs.as_ref().map(|dirs| dirs.config_dir()),
            "config",
        )?;
        let data_dir = resolve_directory(
            ENV_DATA_DIR,
            project_dirs.data_dir(),
            LEGACY_ENV_DATA_DIR,
            legacy_dirs.as_ref().map(|dirs| dirs.data_dir()),
            "data",
        )?;
        let cache_dir = resolve_directory(
            ENV_CACHE_DIR,
            project_dirs.cache_dir(),
            LEGACY_ENV_CACHE_DIR,
            legacy_dirs.as_ref().map(|dirs| dirs.cache_dir()),
            "cache",
        )?;

        let share_dir = resolve_share_dir(&project_dirs, legacy_dirs.as_ref())?;
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
        let mut dirs = vec![
            self.config_dir.join("shaders"),
            self.data_dir.join("shaders"),
        ];

        // Legacy support: also search the data_dir root for existing setups
        let legacy_data_root = self.data_dir.clone();
        if !dirs.contains(&legacy_data_root) {
            dirs.push(legacy_data_root);
        }

        // Legacy support: also search config_dir/shaders (already in dirs)
        let legacy_config = self.config_dir.join("shaders");
        if !dirs.contains(&legacy_config) {
            dirs.push(legacy_config);
        }

        dirs
    }

    pub fn shader_roots(&self) -> Vec<PathBuf> {
        let mut roots = self.shader_user_dirs();
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
        vec![
            self.config_dir.join("playlists"),
            self.data_dir.join("playlists"),
        ]
    }

    pub fn playlist_roots(&self) -> Vec<PathBuf> {
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

fn resolve_directory(
    primary_env: &str,
    primary_default: &Path,
    legacy_env: &str,
    legacy_default: Option<&Path>,
    label: &str,
) -> Result<PathBuf> {
    if let Some(value) = env_override(primary_env) {
        return Ok(value);
    }

    if let Some(value) = env_override(legacy_env) {
        warn!("environment variable {legacy_env} is deprecated; please switch to {primary_env}");
        return Ok(value);
    }

    let primary = primary_default.to_path_buf();
    let mut legacy_candidates = Vec::new();
    if let Some(path) = legacy_default {
        legacy_candidates.push(path.to_path_buf());
    }

    migrate_legacy_directory(primary, &legacy_candidates, label)
        .with_context(|| format!("failed to resolve wallshader {label} directory"))
}

fn resolve_share_dir(
    project_dirs: &ProjectDirs,
    legacy_dirs: Option<&ProjectDirs>,
) -> Result<PathBuf> {
    if let Some(value) = env_override(ENV_SHARE_DIR) {
        return Ok(value);
    }
    if let Some(value) = env_override(LEGACY_ENV_SHARE_DIR) {
        warn!(
            "environment variable {LEGACY_ENV_SHARE_DIR} is deprecated; please switch to {ENV_SHARE_DIR}"
        );
        return Ok(value);
    }

    let primary = default_share_dir(project_dirs);
    if primary.exists() {
        return Ok(primary);
    }

    if let Some(legacy) = legacy_share_dir(legacy_dirs) {
        if legacy.exists() {
            debug!(
                "using legacy share directory at {} until /usr/share/wallshader is populated",
                legacy.display()
            );
            return Ok(legacy);
        }
    }

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

fn migrate_legacy_directory(
    primary: PathBuf,
    legacy_candidates: &[PathBuf],
    label: &str,
) -> Result<PathBuf> {
    if primary.exists() {
        return Ok(primary);
    }

    for legacy in legacy_candidates {
        if legacy == &primary {
            continue;
        }
        if legacy.exists() {
            if let Some(parent) = primary.parent() {
                fs::create_dir_all(parent).with_context(|| {
                    format!(
                        "failed to create parent directory {} while migrating {label} directory",
                        parent.display()
                    )
                })?;
            }

            match fs::rename(legacy, &primary) {
                Ok(_) => {
                    debug!(
                        "migrated legacy {label} directory from {} to {}",
                        legacy.display(),
                        primary.display()
                    );
                    return Ok(primary);
                }
                Err(err) => {
                    warn!(
                        "failed to rename legacy {label} directory from {} to {} ({err}); attempting copy",
                        legacy.display(),
                        primary.display()
                    );
                    copy_recursively(legacy, &primary).with_context(|| {
                        format!(
                            "failed to copy legacy {label} directory from {} to {}",
                            legacy.display(),
                            primary.display()
                        )
                    })?;
                    if let Err(remove_err) = fs::remove_dir_all(legacy) {
                        warn!(
                            "failed to remove legacy {label} directory at {} after migration ({remove_err})",
                            legacy.display()
                        );
                    }
                    debug!(
                        "copied legacy {label} directory from {} to {}",
                        legacy.display(),
                        primary.display()
                    );
                    return Ok(primary);
                }
            }
        }
    }

    Ok(primary)
}

fn copy_recursively(src: &Path, dst: &Path) -> io::Result<()> {
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
                    // Best-effort copy of the linked file contents on non-Unix platforms.
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

#[cfg(target_family = "unix")]
fn default_share_dir(_: &ProjectDirs) -> PathBuf {
    PathBuf::from("/usr/share/wallshader")
}

#[cfg(not(target_family = "unix"))]
fn default_share_dir(project_dirs: &ProjectDirs) -> PathBuf {
    project_dirs.data_dir().to_path_buf()
}

#[cfg(target_family = "unix")]
fn legacy_share_dir(_: Option<&ProjectDirs>) -> Option<PathBuf> {
    Some(PathBuf::from("/usr/share/hyshadew"))
}

#[cfg(not(target_family = "unix"))]
fn legacy_share_dir(legacy_dirs: Option<&ProjectDirs>) -> Option<PathBuf> {
    legacy_dirs.map(|dirs| dirs.data_dir().to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::fs;
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

        assert_eq!(paths.share_dir(), Path::new("/usr/share/wallshader"));
    }

    #[test]
    fn legacy_env_variables_are_supported() {
        let _guard = env_lock().lock().unwrap();
        let root = TempDir::new().unwrap();
        let legacy_config = root.path().join("legacy-config");
        let data_dir = root.path().join("data");
        let cache_dir = root.path().join("cache");
        let share_dir = root.path().join("share");

        fs::create_dir_all(&legacy_config).unwrap();
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&cache_dir).unwrap();
        fs::create_dir_all(&share_dir).unwrap();

        let _legacy_config_guard = EnvGuard::set(super::LEGACY_ENV_CONFIG_DIR, &legacy_config);
        let _config_guard = EnvGuard::clear(ENV_CONFIG_DIR);
        let _data_guard = EnvGuard::set(ENV_DATA_DIR, &data_dir);
        let _cache_guard = EnvGuard::set(ENV_CACHE_DIR, &cache_dir);
        let _share_guard = EnvGuard::set(ENV_SHARE_DIR, &share_dir);

        let paths = AppPaths::discover().unwrap();

        assert_eq!(paths.config_dir(), legacy_config.as_path());
    }

    #[test]
    fn migrates_legacy_directory_when_missing() {
        let root = TempDir::new().unwrap();
        let legacy = root.path().join("hyshadew-config");
        let primary = root.path().join("wallshader-config");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join("settings.toml"), "foo = 1\n").unwrap();

        let result = super::migrate_legacy_directory(
            primary.clone(),
            std::slice::from_ref(&legacy),
            "config",
        )
        .expect("migration succeeded");

        assert_eq!(result, primary);
        assert!(result.join("settings.toml").exists());
        assert!(!legacy.exists());
    }
}
