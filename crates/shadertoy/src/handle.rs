//! Normalizes Shadertoy references and translates CLI-supplied handles into the
//! strongly-typed identifiers that `wallshader` passes around when talking to the
//! pack repository. It leans on `PathResolver` for filesystem semantics while
//! shielding the rest of the crate from the many input shapes users feed the CLI.
//!
//! Functions:
//!
//! - `normalize_shadertoy_reference` trims/rewrites URLs and loose IDs into a
//!   canonical `shadertoy://` form so cache directories and logs stay predictable.
//! - `parse_shader_handle` resolves local paths through `PathResolver` and builds
//!   a `ShaderHandle` that the repository layer can load or refresh.
use anyhow::{bail, Context, Result};

use crate::path::PathResolver;
use crate::ShaderHandle;

pub fn normalize_shadertoy_reference(input: &str) -> Result<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        bail!("shadertoy reference must not be empty");
    }

    if let Some(rest) = trimmed.strip_prefix("shadertoy://") {
        let id = rest.trim();
        if id.is_empty() {
            bail!("shadertoy identifier missing after scheme");
        }
        return Ok(format!("shadertoy://{}", id));
    }

    if trimmed.contains("shadertoy.com") {
        let id_candidate = trimmed
            .split('/')
            .rev()
            .find(|segment| !segment.is_empty())
            .map(|segment| segment.split(&['?', '#'][..]).next().unwrap_or(segment))
            .filter(|id| !id.is_empty());

        if let Some(id) = id_candidate {
            return Ok(format!("shadertoy://{}", id));
        } else {
            bail!("unable to extract shader id from shadertoy url");
        }
    }

    Ok(format!("shadertoy://{}", trimmed))
}

pub fn parse_shader_handle(resolver: &PathResolver, input: &str) -> Result<ShaderHandle> {
    if let Some(stripped) = input.strip_prefix("shadertoy://") {
        let id = stripped.trim();
        if id.is_empty() {
            bail!("shadertoy identifier missing after scheme");
        }
        return Ok(ShaderHandle::ShadertoyId(id.to_string()));
    }

    let normalized = resolver
        .normalize_local_path(input)
        .with_context(|| format!("failed to resolve local shader path '{input}'"))?;
    Ok(ShaderHandle::LocalPack(normalized))
}
