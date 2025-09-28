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

## Multi-Workspace Playlists

Enable playlist mode with `--multi <config>` to drive different shaders per workspace or
output. The configuration format is documented in `SpecMulti.md`, and sample configs live
under `multi/`. A quick way to experiment is:

```
cargo run -p hyshadew -- --multi multi/workspaces.toml
```

Key behaviours:

- Each output resolves to a playlist via workspace or output selectors.
- Workspace changes interrupt any active transition and start a new crossfade using
  `workspace_switch_crossfade` (set to `0` for hard cuts).
- Playlist items can override FPS caps, antialiasing, and opt-in to one-time refreshes.

Runtime telemetry is emitted via `tracing` (see `scripts/launch-local`) and wall-clock logs
with the `[hyshadew]` prefix highlight shader compilation or timing diagnostics.
