use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{bail, Context, Result};
use shadertoy::{PathResolver, ShaderHandle};
use tracing::debug;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryHandle {
    RawPath(PathBuf),
    LocalPack { name: String },
    Shadertoy { id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaylistHandle {
    RawPath(PathBuf),
    Named { name: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchHandle {
    Entry(EntryHandle),
    Playlist(PlaylistHandle),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaylistHandleArg(pub PlaylistHandle);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchHandleArg(pub LaunchHandle);

impl PlaylistHandleArg {
    pub fn into_inner(self) -> PlaylistHandle {
        self.0
    }
}

impl LaunchHandleArg {
    pub fn into_inner(self) -> LaunchHandle {
        self.0
    }

    pub fn inner(&self) -> &LaunchHandle {
        &self.0
    }
}

impl EntryHandle {
    pub fn parse(input: &str) -> Result<Self> {
        let resolver = PathResolver::new()?;
        Self::parse_with_resolver(&resolver, input)
    }

    pub fn parse_with_resolver(resolver: &PathResolver, input: &str) -> Result<Self> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            bail!("shader handle must not be empty");
        }

        if let Some(rest) = trimmed.strip_prefix("shadertoy://") {
            let id = rest.trim();
            if id.is_empty() {
                bail!("shadertoy identifier missing after scheme");
            }
            return Ok(Self::Shadertoy { id: id.to_string() });
        }

        if let Some(rest) = trimmed.strip_prefix("shader://") {
            let name = parse_named_handle(rest, "shader pack")?;
            return Ok(Self::LocalPack { name });
        }

        // Treat anything that expands to an absolute path or contains a slash as a raw path.
        let expanded = resolver
            .expand_path(trimmed)
            .with_context(|| format!("failed to expand shader path '{trimmed}'"))?;
        let slash_present = trimmed.contains('/');
        if slash_present || expanded.is_absolute() {
            let path = absolutise(&expanded, resolver.cwd());
            return Ok(Self::RawPath(path));
        }

        bail!(
            "unable to infer handle type from '{trimmed}'. Use shader://{trimmed} or provide a filesystem path"
        );
    }

    pub fn parse_with_resolver_or_local(resolver: &PathResolver, input: &str) -> Result<Self> {
        match Self::parse_with_resolver(resolver, input) {
            Ok(handle) => Ok(handle),
            Err(original_err) => {
                let trimmed = input.trim();
                if trimmed.is_empty()
                    || trimmed.contains('/')
                    || trimmed.contains("://")
                    || trimmed.starts_with('~')
                    || trimmed.starts_with('.')
                {
                    Err(original_err)
                } else {
                    debug!(
                        handle = %trimmed,
                        "defaulting bare handle to shader://{}",
                        trimmed
                    );
                    Ok(Self::LocalPack {
                        name: trimmed.to_string(),
                    })
                }
            }
        }
    }

    pub fn into_shader_handle(self) -> ShaderHandle {
        match self {
            EntryHandle::RawPath(path) => ShaderHandle::LocalPack(path),
            EntryHandle::LocalPack { name } => ShaderHandle::LocalPack(PathBuf::from(name)),
            EntryHandle::Shadertoy { id } => ShaderHandle::ShadertoyId(id),
        }
    }
}

impl PlaylistHandle {
    pub fn parse(input: &str) -> Result<Self> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            bail!("playlist handle must not be empty");
        }

        if let Some(rest) = trimmed.strip_prefix("playlist://") {
            let name = parse_named_handle(rest, "playlist")?;
            return Ok(Self::Named { name });
        }

        let resolver = PathResolver::new()?;
        let expanded = resolver
            .expand_path(trimmed)
            .with_context(|| format!("failed to expand playlist path '{trimmed}'"))?;
        if trimmed.contains('/') || expanded.is_absolute() {
            let path = absolutise(&expanded, resolver.cwd());
            return Ok(Self::RawPath(path));
        }

        bail!(
            "unable to infer playlist handle from '{trimmed}'. Use playlist://{trimmed} or provide a filesystem path"
        );
    }

    pub fn resolve_path(&self, search_roots: &[PathBuf]) -> Result<PathBuf> {
        match self {
            PlaylistHandle::RawPath(path) => {
                if path.is_absolute() {
                    if path.is_dir() {
                        bail!(
                            "playlist expects a file, not a directory: {}",
                            path.display()
                        );
                    }
                    if path.exists() {
                        return Ok(path.clone());
                    }
                    bail!("playlist file not found: {}", path.display());
                }

                // For relative raw paths, search in playlist directories
                self.search_in_roots(path, search_roots)
            }
            PlaylistHandle::Named { name } => {
                // Add .toml extension if not present
                let filename = if name.ends_with(".toml") {
                    name.clone()
                } else {
                    format!("{}.toml", name)
                };
                let path = PathBuf::from(filename);
                self.search_in_roots(&path, search_roots)
            }
        }
    }

    fn search_in_roots(&self, path: &Path, search_roots: &[PathBuf]) -> Result<PathBuf> {
        use std::path::Component;

        if path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        }) {
            bail!(
                "playlist path does not allow parent or absolute segments: {}",
                path.display()
            );
        }

        for root in search_roots {
            let candidate = root.join(path);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }

        let searched: Vec<String> = search_roots
            .iter()
            .map(|root| root.display().to_string())
            .collect();

        bail!(
            "playlist '{}' not found; searched: {}",
            path.display(),
            searched.join(", ")
        );
    }
}

impl FromStr for PlaylistHandleArg {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        PlaylistHandle::parse(s)
            .map(Self)
            .map_err(|err| err.to_string())
    }
}

impl FromStr for LaunchHandleArg {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let trimmed = s.trim();
        if trimmed.starts_with("playlist://") {
            PlaylistHandle::parse(trimmed)
                .map(LaunchHandle::Playlist)
                .map(LaunchHandleArg)
                .map_err(|err| err.to_string())
        } else {
            EntryHandle::parse(trimmed)
                .map(LaunchHandle::Entry)
                .map(LaunchHandleArg)
                .map_err(|err| err.to_string())
        }
    }
}

impl fmt::Display for PlaylistHandleArg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            PlaylistHandle::RawPath(path) => write!(f, "{}", path.display()),
            PlaylistHandle::Named { name } => write!(f, "playlist://{name}"),
        }
    }
}

impl fmt::Display for LaunchHandleArg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            LaunchHandle::Entry(entry) => match entry {
                EntryHandle::RawPath(path) => write!(f, "{}", path.display()),
                EntryHandle::LocalPack { name } => write!(f, "shader://{name}"),
                EntryHandle::Shadertoy { id } => write!(f, "shadertoy://{id}"),
            },
            LaunchHandle::Playlist(playlist) => PlaylistHandleArg(playlist.clone()).fmt(f),
        }
    }
}

fn absolutise(path: &PathBuf, cwd: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.clone();
    }
    cwd.join(path)
}

fn parse_named_handle(input: &str, kind: &str) -> Result<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        bail!("{kind} handle must include a name, e.g. {kind}://demo");
    }
    if trimmed.contains('/') {
        bail!("{kind} handle names must not contain '/': {trimmed}");
    }
    Ok(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_shadertoy() {
        let handle = EntryHandle::parse("shadertoy://abc").unwrap();
        assert_eq!(handle, EntryHandle::Shadertoy { id: "abc".into() });
    }

    #[test]
    fn parses_shader_scheme() {
        let handle = EntryHandle::parse("shader://demo").unwrap();
        assert_eq!(
            handle,
            EntryHandle::LocalPack {
                name: "demo".into()
            }
        );
    }

    #[test]
    fn raw_paths_require_slash() {
        let handle = EntryHandle::parse("./packs/demo").unwrap();
        match handle {
            EntryHandle::RawPath(path) => assert!(path.ends_with("packs/demo")),
            _ => panic!("expected raw path"),
        }
    }

    #[test]
    fn bare_names_error() {
        let err = EntryHandle::parse("demo").unwrap_err();
        assert!(err.to_string().contains("shader://demo"));
    }

    #[test]
    fn playlist_named() {
        let handle = PlaylistHandle::parse("playlist://demo").unwrap();
        assert_eq!(
            handle,
            PlaylistHandle::Named {
                name: "demo".into()
            }
        );
    }

    #[test]
    fn playlist_raw_path() {
        let handle = PlaylistHandle::parse("./playlists/demo.toml").unwrap();
        match handle {
            PlaylistHandle::RawPath(path) => assert!(path.ends_with("playlists/demo.toml")),
            _ => panic!("expected raw path"),
        }
    }

    #[test]
    fn launch_parses_playlist_scheme() {
        let handle = LaunchHandleArg::from_str("playlist://demo").unwrap();
        match handle.into_inner() {
            LaunchHandle::Playlist(PlaylistHandle::Named { name }) => assert_eq!(name, "demo"),
            _ => panic!("expected playlist launch handle"),
        }
    }

    #[test]
    fn launch_parses_entry_scheme() {
        let handle = LaunchHandleArg::from_str("shader://demo").unwrap();
        match handle.into_inner() {
            LaunchHandle::Entry(EntryHandle::LocalPack { name }) => assert_eq!(name, "demo"),
            _ => panic!("expected entry launch handle"),
        }
    }

    #[test]
    fn parse_with_resolver_or_local_accepts_bare_name() {
        let resolver = PathResolver::with_cwd("/");
        let handle = EntryHandle::parse_with_resolver_or_local(&resolver, "demo").unwrap();
        assert!(matches!(handle, EntryHandle::LocalPack { name } if name == "demo"));
    }
}
