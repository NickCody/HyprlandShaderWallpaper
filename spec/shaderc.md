- [x] Add shaderc dependency and builder integration
- [x] Compile ShaderToy GLSL to SPIR-V before handing to wgpu
- [x] Provide fallback/feature flag for native naga GLSL path
- [ ] Update documentation and troubleshooting guidance
- [ ] Align color pipeline with ShaderToy (sRGB toggles)

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
Document shaderc as a new dependency in `README.md` and `spec/Public.md`, note common build issues (missing libshaderc, needing `shaderc` dev packages), and update troubleshooting sections to mention SPIR-V compilation errors vs. naga validation errors.

## Align color pipeline with ShaderToy (sRGB toggles)
ShaderToy shaders typically write gamma-encoded output directly because the WebGL default framebuffer is non-sRGB. Our renderer currently requests an sRGB swapchain and channel textures, so wgpu applies linear→sRGB conversion when presenting. This double-applies gamma to shaders that already wrote gamma-space colors, producing the brighter/foggier output we observed in testing on 2025-09-29.

**Status (2025-09-29):** No code changes yet—shader output is still assumed linear and the swapchain uses the first sRGB format the adapter exports. Textures load as `Rgba8UnormSrgb` and samplers stay in filterable mode.

**Next agent guidance:**
1. **Config plumbing**: Extend `RendererConfig` with a `ColorSpaceMode` (e.g. `Auto`, `Linear`, `Gamma`) surfaced through the CLI (`--color-space …`) and opt-in manifest override (`render.color_space = "gamma"`). CLI should win, then manifest, then default.
2. **Swapchain + texture formats**: Allow selecting between `Rgba8UnormSrgb` and `Rgba8Unorm` when configuring the surface and placeholder textures. Ensure MSAA resolve targets share the same format.
3. **Shader wrapper**: If we stay on an sRGB surface but the shader asks for gamma output, insert a pre-write `pow(color, vec3(2.2))` (or configurable curve) before `outColor`. Conversely, if the shader wants true linear, avoid extra conversion even when the swapchain is sRGB (let wgpu handle it). Consider emitting helper macros (`SHADERPAPER_OUTPUT_LINEAR(color)`) to keep shader-specific overrides simple.
4. **Textures**: Decide whether channel assets should be sampled in linear or sRGB based on the new mode; potentially expose per-channel overrides in `shader.toml` for advanced packs.
5. **Validation/testing**: Compare against the same shader on Shadertoy for each mode, and capture screenshots or luminance metrics so future regressions are easy to spot.
6. **Docs**: Update README/specs once the feature lands so users know how to pick the desired pipeline.
