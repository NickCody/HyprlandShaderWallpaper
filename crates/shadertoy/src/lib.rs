//! Crate entry point wiring together Shadertoy manifest parsing, path
//! resolution, remote fetching, and repository lookup so `wallshader` and
//! `renderer` consumers can pull everything they need from a single module. The
//! higher-level daemon imports these re-exports to parse CLI handles, locate
//! shaders, and translate ShaderToy payloads into renderer-ready packs without
//! touching submodules directly.
//!
//! Re-exports:
//!
//! - Handle utilities (`normalize_shadertoy_reference`, `parse_shader_handle`)
//!   feed `ShaderRepository`.
//! - Manifest types (`ShaderPackManifest`, `ShaderPass`, `PassInput`, etc.) are
//!   surfaced so renderer bindings can inspect channel declarations.
//! - Local pack helpers (`LocalPack`, `ensure_glsl_sources`, `resolve_entry_source`)
//!   let callers validate disk layouts.
//! - Remote APIs (`ShadertoyClient`, `materialize_shader`, and supporting
//!   structs) expose download + caching control to the CLI layer.
//! - Repository plumbing (`ShaderRepository`, `ShaderSource`, `RemoteShader`)
//!   loads whichever shader the user requested.
//!
//! Types:
//!
//! - `ShaderHandle` distinguishes between `shadertoy://` IDs and local pack
//!   paths so the rest of the crate can dispatch correctly.
mod handle;
mod manifest;
mod pack;
mod path;
mod remote;
mod repository;

pub use handle::{normalize_shadertoy_reference, parse_shader_handle};
pub use manifest::{
    ColorSpace, InputSource, PassInput, PassKind, ShaderPackManifest, ShaderPass, SurfaceAlpha,
};
pub use pack::{ensure_glsl_sources, resolve_entry_source, LocalPack, PackError};
pub use path::PathResolver;
pub use remote::{
    materialize_shader, RenderInput, RenderOutput, RenderPass, ShaderInfo, ShaderPayload,
    ShadertoyClient, ShadertoyConfig,
};
pub use repository::{load_entry_shader, RemoteShader, ShaderRepository, ShaderSource};

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShaderHandle {
    ShadertoyId(String),
    LocalPack(PathBuf),
}

impl ShaderHandle {
    pub fn from_input(input: &str) -> Self {
        if let Some(id) = input.strip_prefix("shadertoy://") {
            Self::ShadertoyId(id.to_string())
        } else {
            Self::LocalPack(PathBuf::from(input))
        }
    }

    pub fn is_local(&self) -> bool {
        matches!(self, Self::LocalPack(_))
    }

    pub fn expects_layout(&self) -> Option<&Path> {
        match self {
            Self::LocalPack(path) => Some(path.as_path()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_shadertoy_scheme() {
        assert_eq!(
            ShaderHandle::from_input("shadertoy://abc"),
            ShaderHandle::ShadertoyId("abc".into())
        );
    }

    #[test]
    fn parses_local_path() {
        assert!(matches!(
            ShaderHandle::from_input("packs/demo"),
            ShaderHandle::LocalPack(path) if path == PathBuf::from("packs/demo")
        ));
    }
}
