use std::borrow::Cow;

use anyhow::{anyhow, Context, Result};
use wgpu::naga::ShaderStage;

use crate::types::ShaderCompiler;

#[cfg(feature = "shaderc")]
use tracing::warn;

/// Compiles the static full-screen triangle vertex shader.
pub(crate) fn compile_vertex_shader(
    device: &wgpu::Device,
    compiler: ShaderCompiler,
) -> Result<wgpu::ShaderModule> {
    compile_glsl(
        device,
        VERTEX_SHADER_GLSL,
        ShaderStage::Vertex,
        "fullscreen triangle vertex",
        compiler,
    )
}

/// Wraps the user shader with our ShaderToy prelude and compiles it as GLSL.
///
/// The wrapped source is dumped to `/tmp/lambdash_wrapped.frag` to aid
/// debugging when compilation fails in `wgpu`.
pub(crate) fn compile_fragment_shader(
    device: &wgpu::Device,
    source: &str,
    compiler: ShaderCompiler,
) -> Result<wgpu::ShaderModule> {
    let wrapped = wrap_shadertoy_fragment(source);

    if let Err(err) = std::fs::write("/tmp/lambdash_wrapped.frag", &wrapped) {
        eprintln!("[lambdash] failed to dump wrapped shader: {err}");
    }

    compile_glsl(
        device,
        &wrapped,
        ShaderStage::Fragment,
        "lambdash fragment",
        compiler,
    )
    .with_context(|| "failed to compile fragment shader")
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
    vec4 _iSurface;
    vec4 _iFill;
    vec4 _iFillWrap;
} ubo;

// Map ShaderToy names to our UBO fields via macros to avoid name clashes.
#define iResolution ubo._iResolution
#define iTime ubo._iTime
#define iTimeDelta ubo._iTimeDelta
#define iFrame ubo._iFrame
#define iMouse ubo._iMouse
#define iDate ubo._iDate
#define iSampleRate ubo._iSampleRate
#define lambdash_mix ubo._iFade
#define iChannelTime ubo._iChannelTime
#define iChannelResolution ubo._iChannelResolution

layout(set = 1, binding = 0) uniform texture2D lambdash_channel0_texture;
layout(set = 1, binding = 1) uniform sampler lambdash_channel0_sampler;
layout(set = 1, binding = 2) uniform texture2D lambdash_channel1_texture;
layout(set = 1, binding = 3) uniform sampler lambdash_channel1_sampler;
layout(set = 1, binding = 4) uniform texture2D lambdash_channel2_texture;
layout(set = 1, binding = 5) uniform sampler lambdash_channel2_sampler;
layout(set = 1, binding = 6) uniform texture2D lambdash_channel3_texture;
layout(set = 1, binding = 7) uniform sampler lambdash_channel3_sampler;

#define iChannel0 sampler2D(lambdash_channel0_texture, lambdash_channel0_sampler)
#define iChannel1 sampler2D(lambdash_channel1_texture, lambdash_channel1_sampler)
#define iChannel2 sampler2D(lambdash_channel2_texture, lambdash_channel2_sampler)
#define iChannel3 sampler2D(lambdash_channel3_texture, lambdash_channel3_sampler)
#define lambdash_Surface ubo._iSurface
#define lambdash_Fill ubo._iFill
#define lambdash_FillWrap ubo._iFillWrap

vec4 lambdash_gl_FragCoord;
#define gl_FragCoord lambdash_gl_FragCoord
";

/// GLSL epilogue that remaps coordinates and delegates to `mainImage`.
const FOOTER: &str = r"void main() {
    // Capture the real builtin gl_FragCoord, then remap to ShaderToy's bottom-left origin.
    // We temporarily undef the macro so we can read the hardware builtin.
    #undef gl_FragCoord
    vec2 builtinFC = vec2(gl_FragCoord.x, gl_FragCoord.y);
    #define gl_FragCoord lambdash_gl_FragCoord

    vec2 mapped = vec2(
        builtinFC.x * lambdash_Fill.x + lambdash_Fill.z,
        (lambdash_Surface.y - builtinFC.y) * lambdash_Fill.y + lambdash_Fill.w
    );

    bool outside = mapped.x < 0.0 || mapped.y < 0.0 || mapped.x >= iResolution.x || mapped.y >= iResolution.y;

    if (lambdash_FillWrap.x > 0.0) {
        mapped.x = mod(mapped.x, lambdash_FillWrap.x);
        if (mapped.x < 0.0) {
            mapped.x += lambdash_FillWrap.x;
        }
        outside = false;
    }

    if (lambdash_FillWrap.y > 0.0) {
        mapped.y = mod(mapped.y, lambdash_FillWrap.y);
        if (mapped.y < 0.0) {
            mapped.y += lambdash_FillWrap.y;
        }
        outside = false;
    }

    if (outside) {
        outColor = vec4(0.0);
        return;
    }

    vec2 fragCoord = mapped;
    lambdash_gl_FragCoord = vec4(fragCoord, 0.0, 1.0);

    vec4 color = vec4(0.0);
    mainImage(color, fragCoord);
    outColor = vec4(color.rgb * lambdash_mix, lambdash_mix);
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

fn compile_glsl(
    device: &wgpu::Device,
    source: &str,
    stage: ShaderStage,
    label: &'static str,
    compiler: ShaderCompiler,
) -> Result<wgpu::ShaderModule> {
    match compiler {
        ShaderCompiler::Shaderc => compile_with_shaderc(device, source, stage, label),
        ShaderCompiler::NagaGlsl => Ok(device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(label),
            source: wgpu::ShaderSource::Glsl {
                shader: Cow::Owned(source.to_owned()),
                stage,
                defines: &[],
            },
        })),
    }
}

#[cfg(feature = "shaderc")]
fn compile_with_shaderc(
    device: &wgpu::Device,
    source: &str,
    stage: ShaderStage,
    label: &'static str,
) -> Result<wgpu::ShaderModule> {
    use shaderc::{
        CompileOptions, Compiler, EnvVersion, OptimizationLevel, ShaderKind, SourceLanguage,
        TargetEnv,
    };

    let compiler = Compiler::new().context("failed to create shaderc compiler")?;
    let mut options = CompileOptions::new().context("failed to create shaderc options")?;
    options.set_source_language(SourceLanguage::GLSL);
    options.set_target_env(TargetEnv::Vulkan, EnvVersion::Vulkan1_1 as u32);
    options.set_optimization_level(if cfg!(debug_assertions) {
        OptimizationLevel::Zero
    } else {
        OptimizationLevel::Performance
    });

    let shader_kind = match stage {
        ShaderStage::Vertex => ShaderKind::Vertex,
        ShaderStage::Fragment => ShaderKind::Fragment,
        ShaderStage::Compute => ShaderKind::Compute,
        other => return Err(anyhow!("unsupported shader stage: {other:?}")),
    };

    let artifact = compiler
        .compile_into_spirv(source, shader_kind, label, "main", Some(&options))
        .with_context(|| format!("shaderc failed to compile {label}"))?;

    let warnings = artifact.get_warning_messages();
    if !warnings.is_empty() {
        warn!(label = label, warnings = %warnings, "shaderc emitted warnings");
    }

    let spirv = artifact.as_binary().to_vec();
    Ok(device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::SpirV(Cow::Owned(spirv)),
    }))
}

#[cfg(not(feature = "shaderc"))]
fn compile_with_shaderc(
    _device: &wgpu::Device,
    _source: &str,
    _stage: ShaderStage,
    label: &'static str,
) -> Result<wgpu::ShaderModule> {
    anyhow::bail!(
        "shaderc support was not enabled at build time; cannot compile {}",
        label
    );
}

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
        assert!(wrapped.contains("lambdash_mix"));
    }
}
