//! Translates validated ShaderToy-style manifests into runtime bindings so the Wayland
//! daemon (`run.rs`, `multi.rs`) can hand the renderer crate a complete picture of what to
//! load, while leaning on `renderer` for channel setup and `shadertoy` for manifest
//! semantics. It keeps CLI-initiated or playlist-driven shader loads in sync with GPU-side
//! expectations.
//!
//! Types:
//!
//! - `ChannelBindingReport` bundles the `renderer::ChannelBindings` alongside
//!   diagnosable issues for later logging.
//! - `ChannelBindingIssue` and `ChannelBindingIssueKind` label per-channel failures so
//!   warnings stay actionable.
//!
//! Functions:
//!
//! - `channel_bindings_from_pack` walks manifest inputs into concrete resources.
//! - `find_cubemap_face` probes pack directories for cubemap face files.
//! - `map_manifest_*` helpers resolve color/alpha preferences with CLI overrides through
//!   `resolve_color_space`.

use std::path::{Path, PathBuf};

use renderer::{
    ChannelBindings, ColorSpaceMode, SurfaceAlpha as RendererSurfaceAlpha, CUBEMAP_FACE_STEMS,
};
use shadertoy::{
    ColorSpace as ManifestColorSpace, InputSource, LocalPack, SurfaceAlpha as ManifestSurfaceAlpha,
};
use tracing::warn;

#[derive(Debug, Clone)]
pub struct ChannelBindingReport {
    pub bindings: ChannelBindings,
    pub issues: Vec<ChannelBindingIssue>,
}

impl ChannelBindingReport {
    pub fn log_warnings(&self) {
        for issue in &self.issues {
            issue.log_warning();
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChannelBindingIssue {
    pub pass: String,
    pub channel: u8,
    pub kind: ChannelBindingIssueKind,
}

impl ChannelBindingIssue {
    fn log_warning(&self) {
        match &self.kind {
            ChannelBindingIssueKind::TextureMissing { path } => warn!(
                pass = %self.pass,
                channel = self.channel,
                path = %path.display(),
                "channel texture not found on disk"
            ),
            ChannelBindingIssueKind::TextureAssignFailed { path, error } => warn!(
                pass = %self.pass,
                channel = self.channel,
                path = %path.display(),
                error = %error,
                "failed to register texture channel"
            ),
            ChannelBindingIssueKind::KeyboardAssignFailed { error } => warn!(
                pass = %self.pass,
                channel = self.channel,
                error = %error,
                "failed to register keyboard channel"
            ),
            ChannelBindingIssueKind::UnsupportedBuffer { name } => warn!(
                pass = %self.pass,
                channel = self.channel,
                buffer = %name,
                "buffer channels are not supported yet"
            ),
            ChannelBindingIssueKind::CubemapDirectoryMissing { directory } => warn!(
                pass = %self.pass,
                channel = self.channel,
                dir = %directory.display(),
                "cubemap directory not found"
            ),
            ChannelBindingIssueKind::CubemapNotDirectory { path } => warn!(
                pass = %self.pass,
                channel = self.channel,
                path = %path.display(),
                "cubemap path is not a directory"
            ),
            ChannelBindingIssueKind::CubemapFaceMissing { directory, face } => warn!(
                pass = %self.pass,
                channel = self.channel,
                dir = %directory.display(),
                face = %face,
                "cubemap face missing"
            ),
            ChannelBindingIssueKind::CubemapAssignFailed { directory, error } => warn!(
                pass = %self.pass,
                channel = self.channel,
                dir = %directory.display(),
                error = %error,
                "failed to register cubemap channel"
            ),
            ChannelBindingIssueKind::UnsupportedAudio { path } => warn!(
                pass = %self.pass,
                channel = self.channel,
                path = %path.display(),
                "audio channels are not supported yet"
            ),
        }
    }
}

#[derive(Debug, Clone)]
pub enum ChannelBindingIssueKind {
    TextureMissing { path: PathBuf },
    TextureAssignFailed { path: PathBuf, error: String },
    KeyboardAssignFailed { error: String },
    UnsupportedBuffer { name: String },
    CubemapDirectoryMissing { directory: PathBuf },
    CubemapNotDirectory { path: PathBuf },
    CubemapFaceMissing { directory: PathBuf, face: String },
    CubemapAssignFailed { directory: PathBuf, error: String },
    UnsupportedAudio { path: PathBuf },
}

pub fn channel_bindings_from_pack(pack: &LocalPack) -> ChannelBindingReport {
    let mut bindings = ChannelBindings::default();
    let mut issues = Vec::new();
    let manifest = pack.manifest();
    let entry_name = &manifest.entry;
    let entry_pass = manifest.passes.iter().find(|pass| &pass.name == entry_name);

    let Some(pass) = entry_pass else {
        warn!(entry = %entry_name, "entry pass missing; no channels bound");
        return ChannelBindingReport { bindings, issues };
    };

    for input in &pass.inputs {
        match &input.source {
            InputSource::Texture { path } => {
                let resolved = if path.is_absolute() {
                    path.clone()
                } else {
                    pack.root().join(path)
                };
                let resolved_for_log = resolved.clone();
                if !resolved_for_log.exists() {
                    issues.push(ChannelBindingIssue {
                        pass: pass.name.clone(),
                        channel: input.channel,
                        kind: ChannelBindingIssueKind::TextureMissing {
                            path: resolved_for_log.clone(),
                        },
                    });
                }
                if let Err(err) = bindings.set_texture(input.channel as usize, resolved) {
                    issues.push(ChannelBindingIssue {
                        pass: pass.name.clone(),
                        channel: input.channel,
                        kind: ChannelBindingIssueKind::TextureAssignFailed {
                            path: resolved_for_log,
                            error: err.to_string(),
                        },
                    });
                }
            }
            InputSource::Keyboard => {
                if let Err(err) = bindings.set_keyboard(input.channel as usize) {
                    issues.push(ChannelBindingIssue {
                        pass: pass.name.clone(),
                        channel: input.channel,
                        kind: ChannelBindingIssueKind::KeyboardAssignFailed {
                            error: err.to_string(),
                        },
                    });
                }
            }
            InputSource::Buffer { name } => {
                issues.push(ChannelBindingIssue {
                    pass: pass.name.clone(),
                    channel: input.channel,
                    kind: ChannelBindingIssueKind::UnsupportedBuffer { name: name.clone() },
                });
            }
            InputSource::Cubemap { directory } => {
                let resolved = if directory.is_absolute() {
                    directory.clone()
                } else {
                    pack.root().join(directory)
                };
                if !resolved.exists() {
                    issues.push(ChannelBindingIssue {
                        pass: pass.name.clone(),
                        channel: input.channel,
                        kind: ChannelBindingIssueKind::CubemapDirectoryMissing {
                            directory: resolved.clone(),
                        },
                    });
                } else if !resolved.is_dir() {
                    issues.push(ChannelBindingIssue {
                        pass: pass.name.clone(),
                        channel: input.channel,
                        kind: ChannelBindingIssueKind::CubemapNotDirectory {
                            path: resolved.clone(),
                        },
                    });
                } else {
                    for face in CUBEMAP_FACE_STEMS {
                        if find_cubemap_face(&resolved, face).is_none() {
                            issues.push(ChannelBindingIssue {
                                pass: pass.name.clone(),
                                channel: input.channel,
                                kind: ChannelBindingIssueKind::CubemapFaceMissing {
                                    directory: resolved.clone(),
                                    face: face.to_string(),
                                },
                            });
                        }
                    }
                }
                if let Err(err) = bindings.set_cubemap(input.channel as usize, resolved.clone()) {
                    issues.push(ChannelBindingIssue {
                        pass: pass.name.clone(),
                        channel: input.channel,
                        kind: ChannelBindingIssueKind::CubemapAssignFailed {
                            directory: resolved,
                            error: err.to_string(),
                        },
                    });
                }
            }
            InputSource::Audio { path } => {
                issues.push(ChannelBindingIssue {
                    pass: pass.name.clone(),
                    channel: input.channel,
                    kind: ChannelBindingIssueKind::UnsupportedAudio { path: path.clone() },
                });
            }
        }
    }

    ChannelBindingReport { bindings, issues }
}

fn find_cubemap_face(directory: &Path, face: &str) -> Option<PathBuf> {
    let target = face.to_ascii_lowercase();
    let entries = std::fs::read_dir(directory).ok()?;
    let mut candidates = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let stem = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().to_ascii_lowercase());
        if matches!(stem.as_deref(), Some(stem) if stem == target) {
            candidates.push(path);
        }
    }
    candidates.sort();
    candidates.into_iter().next()
}

pub fn map_manifest_alpha(alpha: ManifestSurfaceAlpha) -> RendererSurfaceAlpha {
    match alpha {
        ManifestSurfaceAlpha::Opaque => RendererSurfaceAlpha::Opaque,
        ManifestSurfaceAlpha::Transparent => RendererSurfaceAlpha::Transparent,
    }
}

pub fn map_manifest_color(color: ManifestColorSpace) -> ColorSpaceMode {
    match color {
        ManifestColorSpace::Auto => ColorSpaceMode::Auto,
        ManifestColorSpace::Gamma => ColorSpaceMode::Gamma,
        ManifestColorSpace::Linear => ColorSpaceMode::Linear,
    }
}

pub fn resolve_color_space(cli: ColorSpaceMode, manifest: ColorSpaceMode) -> ColorSpaceMode {
    match cli {
        ColorSpaceMode::Auto => match manifest {
            ColorSpaceMode::Auto => {
                tracing::debug!("color space auto -> gamma (Shadertoy default)");
                ColorSpaceMode::Gamma
            }
            other => {
                tracing::debug!(color_space = ?other, "using manifest color space preference");
                other
            }
        },
        other => {
            tracing::debug!(color_space = ?other, "using CLI color space override");
            other
        }
    }
}
