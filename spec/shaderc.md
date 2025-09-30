- [x] Add shaderc dependency and builder integration
- [x] Compile ShaderToy GLSL to SPIR-V before handing to wgpu
- [x] Provide fallback/feature flag for native naga GLSL path
- [x] Update documentation and troubleshooting guidance
- [x] Align color pipeline with ShaderToy (sRGB toggles)

## Add shaderc dependency and builder integration
Use the `shaderc` crate (or a standalone shaderc binary invoked via build.rs) to gain access to the GLSL compiler. Handle optional features for system-installed shaderc vs. vendored builds, and ensure CI images have the required toolchain.

**Status (2025-09-29):** `renderer` now depends on `shaderc 0.10` behind the default `shaderc` feature. `hyshadew` re-exports the same feature set so downstream builds can opt into vendored (`shaderc-build-from-source`) or static (`shaderc-prefer-static-linking`) link modes. `cargo check -p hyshadew` succeeds when shaderc builds from source via CMake.

## Compile ShaderToy GLSL to SPIR-V before handing to wgpu
Replace the current GLSL ingestion step with a shaderc compile: take the wrapped ShaderToy fragment code, compile to SPIR-V, and pass the resulting binary into `Device::create_shader_module`. Capture compiler diagnostics and surface them in logs to aid debugging.

**Status (2025-09-29):** `compile_vertex_shader`/`compile_fragment_shader` now dispatch on `RendererConfig.shader_compiler`. The shaderc path builds SPIR-V binaries (via `wgpu::ShaderSource::SpirV`) and logs warning diagnostics, while the Naga path keeps the existing `ShaderSource::Glsl` flow. Both window and wallpaper GPU state riders propagate the config, so runtime swaps respect the selected backend. `/tmp/shaderpaper_wrapped.frag` dumping is preserved for debugging.

## Provide fallback/feature flag for native naga GLSL path
Keep the current naga pipeline behind a cargo feature or runtime flag in case shaderc is unavailable or problematic. Allow users to opt back into the pure-naga path for testing or environments without shaderc support.

**Status (2025-09-29):** New CLI flag `--shader-compiler {shaderc|naga}` maps to `RendererConfig.shader_compiler`, defaulting to shaderc when the feature exists. The internal enum provides a runtime switch even when both compilers are compiled in.

## Update documentation and troubleshooting guidance
`README.md` now documents the shader compiler flag (`--shader-compiler`) and colour
space controls (`--color-space`, manifest `color_space`). Remaining work for future
agents: produce a dedicated user guide (`docs/`) that walks through manifest
fields, playlist overrides, and troubleshooting examples; keep README as the quick
reference once that guide exists.

## Align color pipeline with ShaderToy (sRGB toggles)
ShaderToy shaders typically write gamma-encoded output directly because the WebGL default framebuffer is non-sRGB. Our renderer currently requests an sRGB swapchain and channel textures, so wgpu applies linear→sRGB conversion when presenting. This double-applies gamma to shaders that already wrote gamma-space colors, producing the brighter/foggier output we observed in testing on 2025-09-29.

**Status (2025-09-29):** Renderer now accepts `ColorSpaceMode` (CLI `--color-space {auto|gamma|linear}` plus manifest `color_space = "…"`). Auto defaults to gamma-style rendering that matches Shadertoy by selecting non-sRGB swapchains/textures. Linear mode switches to sRGB swapchains and texture sampling. Playlist swaps propagate the resolved mode, and the wallpaper runtime reinitialises surfaces when the selection changes.

**Next agent guidance:**
1. **Per-item overrides**: Consider extending multi-playlist TOML (and eventually manifests) with per-pass or per-playlist color overrides, honouring the CLI > playlist > manifest > default hierarchy.
2. **Shader wrapper helpers**: Add optional macros or uniforms so advanced shaders can explicitly request gamma↔linear conversions without retooling the pipeline.
3. **Validation tooling**: Capture side-by-side screenshots or numeric comparisons against Shadertoy for both gamma and linear modes to document expected output.
4. **Docs**: Update README/specs to describe the new CLI flag, manifest field, and how defaults behave. Mention the gamma default and the linear option for physically-based workflows.
