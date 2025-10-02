use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppState {
    pub defaults_version: Option<String>,
    pub last_defaults_sync: Option<String>,
    pub flags: Vec<String>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            defaults_version: None,
            last_defaults_sync: None,
            flags: Vec::new(),
        }
    }
}

impl AppState {
    pub fn load_or_default(path: &Path) -> Result<Self> {
        if path.exists() {
            let contents = fs::read_to_string(path)
                .with_context(|| format!("failed to read state file at {}", path.display()))?;
            let state: Self = toml::from_str(&contents)
                .with_context(|| format!("failed to parse state file at {}", path.display()))?;
            Ok(state)
        } else {
            Ok(Self::default())
        }
    }

    pub fn persist(&self, path: &Path) -> Result<()> {
        let dir = path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("state path has no parent: {}", path.display()))?;
        fs::create_dir_all(dir).with_context(|| {
            format!(
                "failed to prepare directory for state file at {}",
                dir.display()
            )
        })?;
        let serialized = toml::to_string_pretty(self)
            .with_context(|| "failed to serialize state file to TOML".to_string())?;
        fs::write(path, serialized)
            .with_context(|| format!("failed to write state file to {}", path.display()))?;
        Ok(())
    }
}
