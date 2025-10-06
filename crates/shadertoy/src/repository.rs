//! Resolves shader handles into ready-to-render packs, hiding the difference
//! between local directories and cached Shadertoy downloads from the rest of
//! WallShader. CLI and playlist code hand it `ShaderHandle`s, while it consults
//! `PathResolver`, `LocalPack`, and `ShadertoyClient` to load manifests, fetch
//! missing assets, and return entry GLSL paths.
//!
//! Types:
//!
//! - `ShaderSource` distinguishes between local packs and cached remote shaders
//!   for callers that need to inspect cache paths.
//! - `RemoteShader` bundles the remote ID, cache directory, and hydrated
//!   `LocalPack` used by renderer bindings.
//! - `ShaderRepository` stores search roots and cache directories and performs
//!   resolution for every handle the daemon encounters.
//!
//! Functions:
//!
//! - `ShaderRepository::resolve` orchestrates local lookup or remote caching
//!   refreshes before returning a `ShaderSource`.
//! - `ShaderRepository::load_local_pack`, `ensure_remote_cached`, and
//!   `load_cached_remote` implement the individual paths with validation.
//! - Free helper `load_entry_shader` lifts the entry GLSL path out of any
//!   resolved source for the renderer bootstrap.
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Result};
use tracing::{debug, warn};

use crate::pack::{ensure_glsl_sources, resolve_entry_source, LocalPack, PackError};
use crate::path::PathResolver;
use crate::remote::ShadertoyClient;
use crate::ShaderHandle;

#[derive(Debug)]
pub enum ShaderSource {
    Local(LocalPack),
    CachedRemote(RemoteShader),
}

#[derive(Debug)]
pub struct RemoteShader {
    pub id: String,
    pub cache_dir: PathBuf,
    pub pack: LocalPack,
}

#[derive(Debug)]
pub struct ShaderRepository {
    local_roots: Vec<PathBuf>,
    cache_root: PathBuf,
}

impl ShaderRepository {
    pub fn new(local_roots: Vec<PathBuf>, cache_root: PathBuf) -> Self {
        Self {
            local_roots,
            cache_root,
        }
    }

    pub fn build(local_roots: Vec<PathBuf>, cache_root: PathBuf) -> Result<(Self, PathResolver)> {
        let repo = Self::new(local_roots, cache_root);
        let resolver = PathResolver::new()?;
        Ok((repo, resolver))
    }

    pub fn build_with_defaults() -> Result<(Self, PathResolver)> {
        Self::build(
            vec![PathBuf::from("shaders")],
            PathBuf::from("cache/shadertoy"),
        )
    }

    pub fn with_defaults() -> Self {
        Self::new(
            vec![PathBuf::from("shaders")],
            PathBuf::from("cache/shadertoy"),
        )
    }

    pub fn resolve(
        &self,
        handle: &ShaderHandle,
        client: Option<&ShadertoyClient>,
        refresh_remote: bool,
    ) -> Result<ShaderSource> {
        match handle {
            ShaderHandle::LocalPack(path) => {
                let pack = self.load_local_pack(path)?;
                Ok(ShaderSource::Local(pack))
            }
            ShaderHandle::ShadertoyId(id) => {
                let remote = if let Some(client) = client {
                    self.ensure_remote_cached(id, client, refresh_remote)?
                } else {
                    self.load_cached_remote(id)?
                };
                Ok(ShaderSource::CachedRemote(remote))
            }
        }
    }

    fn load_local_pack(&self, path: &Path) -> Result<LocalPack> {
        if path.as_os_str().is_empty() {
            return Err(anyhow!("shader pack path must not be empty"));
        }

        debug!(requested = %path.display(), roots = ?self.local_roots, "resolving local shader pack");
        let candidates = if path.is_absolute() || path.exists() {
            vec![path.to_path_buf()]
        } else {
            self.local_roots
                .iter()
                .map(|root| root.join(path))
                .collect()
        };

        for candidate in candidates {
            debug!(candidate = %candidate.display(), "checking shader pack candidate");
            if candidate.exists() {
                match LocalPack::load(&candidate) {
                    Ok(pack) => {
                        ensure_glsl_sources(&pack).map_err(|err| match err {
                            PackError::ManifestValidation(items) => anyhow!(
                                "shader pack '{}' failed validation: {:?}",
                                candidate.display(),
                                items
                            ),
                            other => anyhow!(other),
                        })?;
                        debug!(path = %candidate.display(), "loaded local shader pack");
                        return Ok(pack);
                    }
                    Err(err) => {
                        warn!(path = %candidate.display(), error = %err, "failed to load local shader pack");
                        return Err(anyhow!(err));
                    }
                }
            }
        }

        warn!(requested = %path.display(), roots = ?self.local_roots, "shader pack missing");
        Err(anyhow!(
            "unable to locate shader pack '{}'. searched roots: {:?}",
            path.display(),
            self.local_roots
        ))
    }

    fn ensure_remote_cached(
        &self,
        id: &str,
        client: &ShadertoyClient,
        refresh: bool,
    ) -> Result<RemoteShader> {
        let cache_dir = self.cache_root.join(id);
        if refresh && cache_dir.exists() {
            fs::remove_dir_all(&cache_dir)?;
        }

        if cache_dir.exists() {
            match self.load_cached_remote(id) {
                Ok(remote) => return Ok(remote),
                Err(err) => {
                    // Cache is corrupt; rebuild it.
                    fs::remove_dir_all(&cache_dir)?;
                    tracing::warn!(
                        shader = id,
                        error = %err,
                        "shadertoy cache invalid, refetching"
                    );
                }
            }
        }

        let pack = client.fetch_and_cache(id, &cache_dir)?;
        ensure_glsl_sources(&pack).map_err(|err| anyhow!(err))?;
        Ok(RemoteShader {
            id: id.to_string(),
            cache_dir,
            pack,
        })
    }

    fn load_cached_remote(&self, id: &str) -> Result<RemoteShader> {
        let cache_dir = self.cache_root.join(id);
        if !cache_dir.exists() {
            bail!("no cached shader '{}' found at {}", id, cache_dir.display());
        }
        let pack = LocalPack::load(&cache_dir).map_err(|err| anyhow!(err))?;
        ensure_glsl_sources(&pack).map_err(|err| anyhow!(err))?;
        Ok(RemoteShader {
            id: id.to_string(),
            cache_dir,
            pack,
        })
    }
}

pub fn load_entry_shader(source: &ShaderSource) -> Result<PathBuf> {
    match source {
        ShaderSource::Local(pack) => resolve_entry_source(pack),
        ShaderSource::CachedRemote(remote) => resolve_entry_source(&remote.pack),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{
        ColorSpace, InputSource, PassInput, PassKind, ShaderPackManifest, ShaderPass, SurfaceAlpha,
    };
    use crate::{
        materialize_shader, RenderInput, RenderOutput, RenderPass, ShaderHandle, ShaderInfo,
        ShaderPayload,
    };

    fn create_pack(dir: &Path) {
        let manifest = ShaderPackManifest {
            name: Some("Demo".into()),
            entry: "image".into(),
            surface_alpha: SurfaceAlpha::Opaque,
            color_space: ColorSpace::Auto,
            description: None,
            tags: vec![],
            passes: vec![ShaderPass {
                name: "image".into(),
                kind: PassKind::Image,
                source: PathBuf::from("image.glsl"),
                inputs: vec![PassInput {
                    channel: 0,
                    source: InputSource::Texture {
                        path: PathBuf::from("textures/tex0.png"),
                    },
                }],
            }],
        };
        std::fs::create_dir_all(dir.join("textures")).unwrap();
        std::fs::write(dir.join("shader.toml"), toml::to_string(&manifest).unwrap()).unwrap();
        std::fs::write(dir.join("image.glsl"), "// shader").unwrap();
        std::fs::write(dir.join("textures/tex0.png"), "fake").unwrap();
    }

    #[test]
    fn resolves_local_relative_pack() {
        let temp = tempfile::tempdir().unwrap();
        let packs_root = temp.path().join("shaders");
        std::fs::create_dir_all(&packs_root).unwrap();
        let pack_dir = packs_root.join("demo");
        std::fs::create_dir_all(&pack_dir).unwrap();
        create_pack(&pack_dir);

        let repo = ShaderRepository::new(vec![packs_root], temp.path().join("cache"));
        let handle = ShaderHandle::LocalPack(PathBuf::from("demo"));
        let source = repo
            .resolve(&handle, None, false)
            .expect("resolve shader pack");
        match source {
            ShaderSource::Local(pack) => {
                assert_eq!(pack.root(), pack_dir);
                let entry = load_entry_shader(&ShaderSource::Local(pack.clone())).unwrap();
                assert!(entry.ends_with("image.glsl"));
            }
            _ => panic!("expected shader pack"),
        }
    }

    #[test]
    fn loads_cached_remote_without_client() {
        let temp = tempfile::tempdir().unwrap();
        let cache_root = temp.path().join("cache");
        let shader_id = "abc123";
        let payload = ShaderPayload {
            info: ShaderInfo {
                name: "Remote Demo".into(),
                description: Some("demo".into()),
                tags: vec!["remote".into()],
            },
            render_passes: vec![RenderPass {
                name: "Image".into(),
                pass_type: "image".into(),
                code: "void mainImage(out vec4 fragColor, in vec2 fragCoord) { fragColor = vec4(1.0); }"
                    .into(),
                inputs: vec![RenderInput {
                    id: None,
                    channel: 0,
                    src: Some("/media/tex0.png".into()),
                    channel_type: "texture".into(),
                }],
                outputs: vec![RenderOutput { id: 9, channel: 0 }],
            }],
        };

        let shader_dir = cache_root.join(shader_id);
        materialize_shader(&payload, &shader_dir, |_, destination| {
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(destination, b"stub").unwrap();
            Ok(())
        })
        .expect("materialize remote shader");

        let repo = ShaderRepository::new(vec![], cache_root.clone());
        let handle = ShaderHandle::ShadertoyId(shader_id.to_string());
        let source = repo
            .resolve(&handle, None, false)
            .expect("resolve cached remote");

        let entry = load_entry_shader(&source).expect("entry path");
        assert!(entry.ends_with("image.glsl"));

        match source {
            ShaderSource::CachedRemote(remote) => {
                assert_eq!(remote.id, shader_id);
                assert_eq!(remote.cache_dir, shader_dir);
            }
            _ => panic!("expected cached remote"),
        }
    }
}
