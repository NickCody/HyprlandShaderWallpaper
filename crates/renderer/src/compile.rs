use std::borrow::Cow;

use anyhow::Result;
use wgpu::naga::ShaderStage;

/// Compiles the static full-screen triangle vertex shader.
pub(crate) fn compile_vertex_shader(device: &wgpu::Device) -> Result<wgpu::ShaderModule> {
    Ok(device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("fullscreen triangle vertex"),
        source: wgpu::ShaderSource::Glsl {
            shader: Cow::Borrowed(VERTEX_SHADER_GLSL),
            stage: ShaderStage::Vertex,
            defines: &[],
        },
    }))
}

/// Wraps the user shader with our ShaderToy prelude and compiles it as GLSL.
///
/// The wrapped source is dumped to `/tmp/shaderpaper_wrapped.frag` to aid
/// debugging when compilation fails in `wgpu`.
pub(crate) fn compile_fragment_shader(
    device: &wgpu::Device,
    source: &str,
) -> Result<wgpu::ShaderModule> {
    let wrapped = wrap_shadertoy_fragment(source);

    if let Err(err) = std::fs::write("/tmp/shaderpaper_wrapped.frag", &wrapped) {
        eprintln!("[shaderpaper] failed to dump wrapped shader: {err}");
    }

    Ok(device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("shaderpaper fragment"),
        source: wgpu::ShaderSource::Glsl {
            shader: Cow::Owned(wrapped),
            stage: ShaderStage::Fragment,
            defines: &[],
        },
    }))
}

/// Produces a self-contained GLSL fragment shader from raw ShaderToy code.
///
/// Steps performed:
///
/// 1. Strip `#version` directives and ShaderToy uniform declarations so we can
///    inject our own definitions.
/// 2. Prepend [`HEADER`] which declares the uniform block, sampler bindings, and
///    macro aliases.
/// 3. Append [`FOOTER`] which remaps `gl_FragCoord`, calls `mainImage`, and
///    writes to `outColor`.
fn wrap_shadertoy_fragment(source: &str) -> String {
    let mut sanitized = String::new();
    let mut skipped_version = false;
    let mut sanitized_lines = Vec::new();
    for line in source.lines() {
        if !skipped_version && line.trim_start().starts_with("#version") {
            skipped_version = true;
            continue;
        }
        let trimmed = line.trim_start();
        let should_skip_uniform = trimmed.starts_with("uniform ")
            && (trimmed.contains("iResolution")
                || trimmed.contains("iTimeDelta")
                || trimmed.contains("iTime")
                || trimmed.contains("iFrame")
                || trimmed.contains("iMouse")
                || trimmed.contains("iDate")
                || trimmed.contains("iSampleRate")
                || trimmed.contains("iChannelTime")
                || trimmed.contains("iChannelResolution")
                || trimmed.contains("iChannel0")
                || trimmed.contains("iChannel1")
                || trimmed.contains("iChannel2")
                || trimmed.contains("iChannel3"));
        if should_skip_uniform {
            continue;
        }
        sanitized_lines.push(line);
    }

    for line in sanitized_lines {
        sanitized.push_str(line);
        sanitized.push('\n');
    }

    format!(
        "{HEADER}\n#line 1\n{sanitized}{FOOTER}",
        sanitized = sanitized
    )
}

/// GLSL prologue injected ahead of every ShaderToy fragment shader.
///
/// The uniform block layout must match [`ShadertoyUniforms`] in `gpu.rs`. Note that we keep
/// `_iResolution` as a vec3 but reserve the fourth float for the mirrored
/// `iTime`, ensuring the shader can animate even if vec3 padding is collapsed.
const HEADER: &str = r"#version 450
layout(location = 0) in vec2 v_uv;
layout(location = 0) out vec4 outColor;

layout(std140, set = 0, binding = 0) uniform ShaderParams {
    vec3 _iResolution;
    float _iTime;
    float _iTimeDelta;
    int _iFrame;
    float _padding0;
    vec4 _iMouse;
    vec4 _iDate;
    float _iSampleRate;
    float _iFade;
    vec2 _padding1;
    float _iChannelTime[4];
    vec3 _iChannelResolution[4];
} ubo;

// Map ShaderToy names to our UBO fields via macros to avoid name clashes.
#define iResolution ubo._iResolution
#define iTime ubo._iTime
#define iTimeDelta ubo._iTimeDelta
#define iFrame ubo._iFrame
#define iMouse ubo._iMouse
#define iDate ubo._iDate
#define iSampleRate ubo._iSampleRate
#define shaderpaper_mix ubo._iFade
#define iChannelTime ubo._iChannelTime
#define iChannelResolution ubo._iChannelResolution

layout(set = 1, binding = 0) uniform texture2D shaderpaper_channel0_texture;
layout(set = 1, binding = 1) uniform sampler shaderpaper_channel0_sampler;
layout(set = 1, binding = 2) uniform texture2D shaderpaper_channel1_texture;
layout(set = 1, binding = 3) uniform sampler shaderpaper_channel1_sampler;
layout(set = 1, binding = 4) uniform texture2D shaderpaper_channel2_texture;
layout(set = 1, binding = 5) uniform sampler shaderpaper_channel2_sampler;
layout(set = 1, binding = 6) uniform texture2D shaderpaper_channel3_texture;
layout(set = 1, binding = 7) uniform sampler shaderpaper_channel3_sampler;

#define iChannel0 sampler2D(shaderpaper_channel0_texture, shaderpaper_channel0_sampler)
#define iChannel1 sampler2D(shaderpaper_channel1_texture, shaderpaper_channel1_sampler)
#define iChannel2 sampler2D(shaderpaper_channel2_texture, shaderpaper_channel2_sampler)
#define iChannel3 sampler2D(shaderpaper_channel3_texture, shaderpaper_channel3_sampler)

vec4 shaderpaper_gl_FragCoord;
#define gl_FragCoord shaderpaper_gl_FragCoord
";

/// GLSL epilogue that remaps coordinates and delegates to `mainImage`.
const FOOTER: &str = r"void main() {
    // Capture the real builtin gl_FragCoord, then remap to ShaderToy's bottom-left origin.
    // We temporarily undef the macro so we can read the hardware builtin.
    #undef gl_FragCoord
    vec2 builtinFC = vec2(gl_FragCoord.x, gl_FragCoord.y);
    #define gl_FragCoord shaderpaper_gl_FragCoord

    vec2 fragCoord = vec2(builtinFC.x, iResolution.y - builtinFC.y);
    shaderpaper_gl_FragCoord = vec4(fragCoord, 0.0, 1.0);

    vec4 color = vec4(0.0);
    mainImage(color, fragCoord);
    outColor = vec4(color.rgb * shaderpaper_mix, shaderpaper_mix);
}
";

/// Minimal full-screen triangle vertex shader.
const VERTEX_SHADER_GLSL: &str = r"#version 450
layout(location = 0) out vec2 v_uv;

const vec2 positions[3] = vec2[3](
    vec2(-1.0, -3.0),
    vec2(3.0, 1.0),
    vec2(-1.0, 1.0)
);

void main() {
    uint vertex_index = uint(gl_VertexIndex);
    vec2 pos = positions[vertex_index];
    v_uv = pos * 0.5 + vec2(0.5, 0.5);
    gl_Position = vec4(pos, 0.0, 1.0);
}
";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_strips_shadertoy_uniforms() {
        let source = r#"
            #version 300 es
            uniform float iTime;
            uniform vec3 iResolution;
            void mainImage(out vec4 fragColor, in vec2 fragCoord) {
                fragColor = vec4(fragCoord, 0.0, 1.0);
            }
        "#;

        let wrapped = wrap_shadertoy_fragment(source);
        assert!(!wrapped.contains("uniform float iTime"));
        assert!(!wrapped.contains("uniform vec3 iResolution"));
        assert!(wrapped.contains("mainImage"));
        assert!(wrapped.contains("shaderpaper_mix"));
    }
}
