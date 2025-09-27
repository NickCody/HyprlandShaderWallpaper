mod manifest;
mod pack;
mod remote;
mod repository;

pub use manifest::{InputSource, PassInput, PassKind, ShaderPackManifest, ShaderPass};
pub use pack::{ensure_glsl_sources, resolve_entry_source, LocalPack, PackError};
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
