use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ShaderPackManifest {
    pub name: Option<String>,
    #[serde(default = "default_entry")]
    pub entry: String,
    #[serde(default)]
    pub surface_alpha: SurfaceAlpha,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub passes: Vec<ShaderPass>,
}

fn default_entry() -> String {
    "image".to_string()
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ShaderPass {
    pub name: String,
    #[serde(default = "default_pass_kind")]
    pub kind: PassKind,
    pub source: PathBuf,
    #[serde(default)]
    pub inputs: Vec<PassInput>,
}

fn default_pass_kind() -> PassKind {
    PassKind::Image
}

#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PassKind {
    Image,
    Buffer,
    Sound,
    Cubemap,
}

impl Default for PassKind {
    fn default() -> Self {
        Self::Image
    }
}

#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SurfaceAlpha {
    Opaque,
    Transparent,
}

impl Default for SurfaceAlpha {
    fn default() -> Self {
        Self::Opaque
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct PassInput {
    pub channel: u8,
    #[serde(flatten)]
    pub source: InputSource,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum InputSource {
    Buffer { name: String },
    Texture { path: PathBuf },
    Cubemap { directory: PathBuf },
    Audio { path: PathBuf },
}

impl ShaderPackManifest {
    pub fn validate(&self) -> Vec<String> {
        let mut issues = Vec::new();
        if self.passes.is_empty() {
            issues.push("manifest must declare at least one pass".to_string());
        }
        if self.passes.iter().all(|pass| pass.name != self.entry) {
            issues.push(format!("entry pass '{}' not found", self.entry));
        }
        for pass in &self.passes {
            for input in &pass.inputs {
                if input.channel > 3 {
                    issues.push(format!(
                        "pass '{}' uses channel {} which exceeds Shadertoy limit",
                        pass.name, input.channel
                    ));
                }
                if let InputSource::Buffer { name } = &input.source {
                    if self.passes.iter().all(|candidate| &candidate.name != name) {
                        issues.push(format!(
                            "pass '{}' references buffer '{}' which is undefined",
                            pass.name, name
                        ));
                    }
                }
            }
        }
        issues
    }
}
