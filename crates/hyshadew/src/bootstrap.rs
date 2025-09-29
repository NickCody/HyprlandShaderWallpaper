use anyhow::{bail, Result};
use renderer::RendererConfig;
use shadertoy::ShaderHandle;

use crate::cli::Args;

pub fn resolve_shader_handle(args: &Args) -> Result<ShaderHandle> {
    if let Some(ref shadertoy) = args.shadertoy {
        let normalized = normalize_shadertoy_reference(shadertoy)?;
        return Ok(ShaderHandle::from_input(&normalized));
    }

    let raw = args
        .shader
        .as_ref()
        .cloned()
        .unwrap_or_else(|| "local-demo".to_string());
    Ok(ShaderHandle::from_input(&raw))
}

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

pub fn parse_surface_size(spec: &str) -> Result<(u32, u32)> {
    let trimmed = spec.trim();
    let (width, height) = trimmed
        .split_once(['x', 'X', '×'])
        .ok_or_else(|| anyhow::anyhow!("expected WxH format, e.g. 1920x1080"))?;

    let width: u32 = width
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid width in size specification"))?;
    let height: u32 = height
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid height in size specification"))?;

    if width == 0 || height == 0 {
        anyhow::bail!("surface dimensions must be greater than zero");
    }

    Ok((width, height))
}

pub struct SingleRunConfig {
    pub renderer_config: RendererConfig,
}
