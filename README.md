# Hyprland Shader Wallpaper (hyshadew)

Hyprland Shader Wallpaper (code-named ShaderPaper) is a Rust-based wallpaper engine
for Wayland compositors. It renders ShaderToy-compatible GPU shaders as live
backgrounds and supports both remote ShaderToy content and local shader packs.

## Workspace Layout

- `crates/hyshadew`: Daemon entry point and CLI that orchestrates rendering.
- `crates/renderer`: Rendering abstraction that manages shader wrapping and frame uniforms.
- `crates/shadertoy`: Integration layer for ShaderToy downloads, caching, and manifest validation.
- `local-shaders/`: User-provided shader packs mirroring ShaderToy render pass structure.

## Getting Started

```
cargo run -p hyshadew -- --window --shadertoy https://www.shadertoy.com/view/3dXyWj
```

Provide a valid ShaderToy API key via `--shadertoy-api-key` or the `SHADERTOY_API_KEY`
environment variable to fetch remote shaders on first use. Use `--window` to open a
preview window, or omit it to render directly onto the wallpaper surface.

