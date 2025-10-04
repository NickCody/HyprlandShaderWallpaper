use std::path::PathBuf;

use crate::paths::AppPaths;

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
