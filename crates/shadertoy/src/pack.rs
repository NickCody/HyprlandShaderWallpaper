//! Wraps a shader pack directory so `repository` and `remote` callers can load
//! manifests, locate GLSL sources, and resolve asset paths consistently. It
//! keeps filesystem validation centralized while leaving higher layers to decide
//! whether the pack came from disk or a Shadertoy download.
//!
//! Types:
//!
//! - `PackError` classifies manifest parsing, validation, and I/O failures for
//!   error reporting in `wallshader` telemetry.
//! - `LocalPack` stores the resolved root directory and parsed `ShaderPackManifest`
//!   for later traversal by repository and renderer glue.
//!
//! Functions:
//!
//! - `LocalPack::load` reads `shader.toml`, validates it, and returns a
//!   filesystem-backed handle.
//! - `LocalPack::asset_path` and `passes` expose helper views used during
//!   renderer binding construction.
//! - `resolve_entry_source` locates the entry pass GLSL the renderer should
//!   compile first.
//! - `ensure_glsl_sources` confirms every declared pass has a file on disk so
//!   later compilation errors point at shader code, not missing assets.
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use thiserror::Error;

use crate::manifest::{InputSource, ShaderPackManifest};

#[derive(Debug, Error)]
pub enum PackError {
    #[error("manifest not found at {0}")]
    ManifestMissing(PathBuf),

    #[error("failed to parse manifest: {0}")]
    ManifestParse(#[from] toml::de::Error),

    #[error("manifest validation failed: {0:?}")]
    ManifestValidation(Vec<String>),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone)]
pub struct LocalPack {
    root: PathBuf,
    manifest: ShaderPackManifest,
}

impl LocalPack {
    pub fn load(root: impl AsRef<Path>) -> Result<Self, PackError> {
        let root = root.as_ref().to_path_buf();
        let manifest_path = root.join("shader.toml");
        if !manifest_path.exists() {
            return Err(PackError::ManifestMissing(manifest_path));
        }

        let manifest_raw = fs::read_to_string(&manifest_path)?;
        let manifest: ShaderPackManifest = toml::from_str(&manifest_raw)?;
        let issues = manifest.validate();
        if !issues.is_empty() {
            return Err(PackError::ManifestValidation(issues));
        }

        Ok(Self { root, manifest })
    }

    pub fn root(&self) -> &Path {
        self.root.as_path()
    }

    pub fn manifest(&self) -> &ShaderPackManifest {
        &self.manifest
    }

    pub fn pass_source_path(&self, pass_name: &str) -> Option<PathBuf> {
        self.manifest
            .passes
            .iter()
            .find(|pass| pass.name == pass_name)
            .map(|pass| self.root.join(&pass.source))
    }

    pub fn asset_path(&self, source: &InputSource) -> Option<PathBuf> {
        match source {
            InputSource::Texture { path } | InputSource::Audio { path } => {
                Some(self.root.join(path))
            }
            InputSource::Cubemap { directory } => Some(self.root.join(directory)),
            InputSource::Keyboard | InputSource::Buffer { .. } => None,
        }
    }

    pub fn passes(&self) -> impl Iterator<Item = &crate::manifest::ShaderPass> {
        self.manifest.passes.iter()
    }
}

pub fn resolve_entry_source(pack: &LocalPack) -> Result<PathBuf> {
    let entry = &pack.manifest().entry;
    pack.pass_source_path(entry)
        .ok_or_else(|| anyhow!("entry pass source not found"))
}

pub fn ensure_glsl_sources(pack: &LocalPack) -> Result<Vec<PathBuf>, PackError> {
    let mut missing = Vec::new();
    let mut resolved = Vec::new();
    for pass in pack.passes() {
        let source_path = pack.root().join(&pass.source);
        if !source_path.exists() {
            missing.push(source_path);
        } else {
            resolved.push(source_path);
        }
    }
    if !missing.is_empty() {
        return Err(PackError::ManifestValidation(
            missing
                .into_iter()
                .map(|p| format!("missing shader source: {}", p.display()))
                .collect(),
        ));
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{
        ColorSpace, InputSource, PassInput, PassKind, ShaderPackManifest, ShaderPass, SurfaceAlpha,
    };

    fn write_pack(dir: &Path, manifest: &ShaderPackManifest, extra_files: &[(&str, &str)]) {
        let manifest_str = toml::to_string(manifest).expect("serialize manifest");
        fs::write(dir.join("shader.toml"), manifest_str).expect("write manifest");
        for (path, contents) in extra_files {
            let full_path = dir.join(path);
            if let Some(parent) = full_path.parent() {
                fs::create_dir_all(parent).expect("create dirs");
            }
            fs::write(full_path, contents).expect("write file");
        }
    }

    fn demo_manifest() -> ShaderPackManifest {
        ShaderPackManifest {
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
        }
    }

    #[test]
    fn loads_valid_pack() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = demo_manifest();
        let extra_files = vec![("image.glsl", "// shader"), ("textures/tex0.png", "fake")];
        write_pack(temp.path(), &manifest, &extra_files);

        let pack = LocalPack::load(temp.path()).expect("load pack");
        assert_eq!(pack.manifest().entry, "image");
        assert!(pack.pass_source_path("image").unwrap().exists());
        assert!(pack
            .asset_path(&InputSource::Texture {
                path: PathBuf::from("textures/tex0.png"),
            })
            .unwrap()
            .exists());
        ensure_glsl_sources(&pack).expect("sources exist");
    }

    #[test]
    fn detects_missing_shader_source() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = demo_manifest();
        write_pack(temp.path(), &manifest, &[("textures/tex0.png", "fake")]);

        let pack = LocalPack::load(temp.path()).expect("load pack");
        let err = ensure_glsl_sources(&pack).unwrap_err();
        assert!(matches!(err, PackError::ManifestValidation(_)));
    }
}
