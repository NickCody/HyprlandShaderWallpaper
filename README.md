# Hyprland Shader Wallpaper (hyshadew)

Hyprland Shader Wallpaper (code-named ShaderPaper) is a Rust-based wallpaper engine
for Wayland compositors. It renders ShaderToy-compatible GPU shaders as live
backgrounds and supports both remote ShaderToy content and local shader packs.

## Prerequisites

- Rust toolchain (`rustup` with the `stable` channel) plus the `rustfmt` and `clippy` components
- Wayland development headers (`wayland-protocols`, `wayland-client`, `pkg-config`)
- GPU drivers with Vulkan or OpenGL support (Mesa on Linux works well)
- Optional utilities: [`just`](https://github.com/casey/just) for running the helper recipes below

Install `just` via your package manager or with `cargo install just`.

## Quick Start

```bash
git clone https://github.com/<your-org>/HyprlandShaderWallpaper.git
cd HyprlandShaderWallpaper
just setup
just run-demo
```

`just setup` will install the required toolchain components and pre-fetch crates
so the first build runs quickly. The demo uses a ShaderToy URL; provide your own
API key via `--shadertoy-api-key` or the `SHADERTOY_API_KEY` environment variable
if the shader is not cached locally.

## Install from Git

Install the binary straight from this repository without cloning it:

```bash
cargo install --git https://github.com/<your-org>/HyprlandShaderWallpaper --locked hyshadew
```

Pin `--tag vX.Y.Z` once release tags exist; during development you can substitute
`--branch main`. To reinstall after updates, add `--force`.

If you already have the repository checked out, the same command works locally:

```bash
cargo install --path crates/hyshadew --locked --force hyshadew
```

The installed `hyshadew` binary accepts the same CLI flags documented below, so you
can run `hyshadew --window --shadertoy https://www.shadertoy.com/view/3dXyWj` from
any directory.

## Workspace Layout

- `crates/hyshadew`: Daemon entry point and CLI that orchestrates rendering.
- `crates/renderer`: Rendering abstraction that manages shader wrapping and frame uniforms.
- `crates/shadertoy`: Integration layer for ShaderToy downloads, caching, and manifest validation.
- `local-shaders/`: User-provided shader packs mirroring ShaderToy render pass structure.

## Development Tasks

The repository includes a `justfile` with common workflows:

```
just check      # fmt + clippy over the workspace
just run-demo   # windowed ShaderToy demo
just run-multi  # playlist sampler using multi/workspaces.toml
```

Run `just --list` to discover additional recipes as they land.

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

## Release Notes

See `CHANGELOG.md` for released versions and upgrade highlights.
