- [ ] Add shaderc dependency and builder integration
- [ ] Compile ShaderToy GLSL to SPIR-V before handing to wgpu
- [ ] Provide fallback/feature flag for native naga GLSL path
- [ ] Update documentation and troubleshooting guidance

## Add shaderc dependency and builder integration
Use the `shaderc` crate (or a standalone shaderc binary invoked via build.rs) to gain access to the GLSL compiler. Handle optional features for system-installed shaderc vs. vendored builds, and ensure CI images have the required toolchain.

## Compile ShaderToy GLSL to SPIR-V before handing to wgpu
Replace the current GLSL ingestion step with a shaderc compile: take the wrapped ShaderToy fragment code, compile to SPIR-V, and pass the resulting binary into `Device::create_shader_module`. Capture compiler diagnostics and surface them in logs to aid debugging.

## Provide fallback/feature flag for native naga GLSL path
Keep the current naga pipeline behind a cargo feature or runtime flag in case shaderc is unavailable or problematic. Allow users to opt back into the pure-naga path for testing or environments without shaderc support.

## Update documentation and troubleshooting guidance
Document shaderc as a new dependency in `README.md` and `spec/Public.md`, note common build issues (missing libshaderc, needing `shaderc` dev packages), and update troubleshooting sections to mention SPIR-V compilation errors vs. naga validation errors.
