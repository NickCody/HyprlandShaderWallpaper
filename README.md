# Lambda Shader (lambdash)

Lambda Shader is a Rust-based wallpaper engine
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
git clone https://github.com/NickCody/HyprlandShaderWallpaper.git
cd HyprlandShaderWallpaper
just setup
just run-demo
```

`just setup` will install the required toolchain components and pre-fetch crates
so the first build runs quickly. The demo uses a ShaderToy URL; provide your own
API key via `--shadertoy-api-key` or the `SHADERTOY_API_KEY` environment variable
if the shader is not cached locally.

## Color Space & Gamma Handling

By default Lambda Shader assumes ShaderToy-style gamma output: it creates a non-sRGB
swapchain and binds textures without automatic colour conversion, matching the
WebGL defaults used on shadertoy.com. You can override this at several levels:

- **CLI:** `--color-space {auto|gamma|linear}`. `auto` behaves like ShaderToy
  (`gamma`), while `linear` requests sRGB swapchains/textures so physically based
  shaders can output linear light.
- **Manifest:** Local packs may set `color_space = "gamma"` (or `"linear"`) in
  `shader.toml`. CLI overrides manifest; manifest overrides the default.
- **Playlists:** Multi-playlist runs inherit the same hierarchy—global CLI flag
  dominates, otherwise each pack’s manifest decides the swapchain format.

When switching between modes Lambda Shader rebuilds GPU resources so playlists can mix
gamma-authored and linear-authored content without restarting the daemon.

## Install from Git

Install the binary straight from this repository without cloning it:

```bash
cargo install \
  --git https://github.com/NickCody/HyprlandShaderWallpaper \
  --tag v0.9.1 \
  --locked \
  lambdash
```

During active development you can substitute `--branch main` to follow the latest
changes. Add `--force` to reinstall after pulling a new tag.

If you already have the repository checked out, the same command works locally:

```bash
cargo install --path crates/lambdash --locked --force lambdash
```

The installed `lambdash` binary accepts the same CLI flags documented below, so you
can run `lambdash --window --shadertoy https://www.shadertoy.com/view/3dXyWj` from
any directory.

## Installer Script

Prefer a scripted setup? Use the curl-friendly installer. It clones the
upstream repository, runs `cargo install`, and copies the repo's
`shaders/` tree into your wallpaper data directory without requiring
root:

```bash
bash -c "$(curl -fsSL https://raw.githubusercontent.com/NickCody/HyprlandShaderWallpaper/main/scripts/install.sh)"
```

By default this one-liner grabs `main` from GitHub, overwriting
`~/.local/share/lambdash/shaders` with the bundled shader packs and
reinstalling the binary. Use `--data-dir` to pick a different destination or
`--system` (with sudo) to install the packs under `/usr/share/lambdash`.
Additional flags help with constrained environments—`--skip-build` reuses an
existing binary, and `--offline` forwards Cargo's offline mode when crates are
already cached. All installer options (including `--prefix` and `--ref`) are
documented via `bash scripts/install.sh --help`. Ensure `cargo`, `git`, and
`tar` are available before running the script.

Working from a local checkout? Run the installer directly so it mirrors *your*
branch instead of cloning GitHub:

```bash
scripts/install.sh --source . --skip-build
```

Pass `--data-dir ~/.local/share/lambdash` (or `--system`) if you need to target a
specific location. Re-run the same command whenever you want to refresh
`shaders/` while iterating on shader packs or playlists.

## Directories and CLI Helpers

Lambda Shader follows the XDG base directory spec. The core locations are:

- Config: `~/.config/lambdash/`
- Data: `~/.local/share/lambdash/`
- Cache: `~/.cache/lambdash/`
- System defaults: `/usr/share/lambdash/` (overridable via `LAMBDASH_SHARE_DIR`)

Set `LAMBDASH_CONFIG_DIR`, `LAMBDASH_DATA_DIR`, and `LAMBDASH_CACHE_DIR` to relocate
any directory. CLI flags always win over environment variables.

Bundled shader packs and sample playlists live under `shaders/` in the
repository. The installer mirrors that directory to your data location, and you
can rerun it any time you want to refresh the packs while developing. Keep your
own user overrides under `~/.config/lambdash/shaders`—the installer never
touches that tree.

Run `lambdash defaults where` to print the resolved config/data/cache/share
paths if you need to double-check the environment.

### Still Frames & Exports

- `--still` renders one frame and pauses the scheduler, keeping the surface alive for previews.
- `--still-time <seconds|auto>` selects the timestamp for still/export captures (auto currently falls back to `0`).
- `--still-export <path.png>` grabs the next frame to disk; the renderer forces window mode for capture.
- `--still-exit <true|false>` controls whether Lambda Shader exits after finishing a still export (defaults to `true`).
  When exporting, Lambda Shader hides the preview window unless `--window` is supplied, so headless
  captures complete without flashing a surface.

### Quality & Adaptive Controls

- `--render-scale <0.25-1.0>` supersamples or downsamples before presenting to the compositor.
- `--fill-method stretch|center:WxH|tile[:XxY]` remaps shader coordinates, enabling letterboxing or tiled repeats.
- `--fps-adaptive` enables cadence throttling when the compositor hides the surface; pair with `--max-fps-occluded <fps>` to cap the hidden refresh rate.
- Existing `--fps <value>` remains the steady-state cap while the surface is visible.

## Packaging Guidance

Downstream packages and automation should mirror the installer’s behaviour:

- Invoke `scripts/install.sh --skip-build --data-dir <dest>` during packaging to
  stage the bundled shader packs (`<dest>/<pack>/`) and playlists (`<dest>/*.toml`).
- When producing system packages (`.deb`, `.rpm`, etc.), call the script with
  `--system` or provide explicit `--prefix`/`--data-dir` flags that match your
  filesystem layout.
- Avoid running as root unless shipping a system-wide package. For user-focused
  bundles (AppImage, Flatpak, etc.) set `LAMBDASH_DATA_DIR` to a writable path
  and run the installer in `--skip-build` mode after the binary is staged.
- CI should execute `cargo test -p lambdash` to cover the installer integration
  test (`install_script_copies_defaults`) and ensure future changes keep the
  script functional.

## Workspace Layout

- `crates/lambdash`: Daemon entry point and CLI that orchestrates rendering.
- `crates/renderer`: Rendering abstraction that manages shader wrapping and frame uniforms.
- `crates/shadertoy`: Integration layer for ShaderToy downloads, caching, and manifest validation.
- `shaders/`: User-provided shader packs mirroring ShaderToy render pass structure.

## Development Tasks

The repository includes a `justfile` with common workflows:

```
just check      # fmt + clippy over the workspace
just run-demo   # windowed ShaderToy demo
just run-playlist  # playlist sampler using workspaces.toml
```

Run `just --list` to discover additional recipes as they land.

## Playlists

Enable playlist mode with `--playlist <file>` to drive different shaders per workspace or
output. The configuration format is documented in `SpecMulti.md`, and sample playlists live
under `shaders/playlists/` in the repo (e.g. `workspaces.toml`) and are copied to `$DATA_DIR/*.toml` by the installer. A quick way to
experiment is:

```
cargo run -p lambdash -- --playlist workspaces.toml
```

Key behaviours:

- Each output resolves to a playlist via workspace or output selectors.
- Workspace changes interrupt any active transition and start a new crossfade using
  `workspace_switch_crossfade` (set to `0` for hard cuts).
- Playlist items can override FPS caps, antialiasing, and opt-in to one-time refreshes.
- Playlist entries now support `mode = "still"` (optionally `still_time = "2s"`) for photo-style slides that render once.

Runtime telemetry is emitted via `tracing` (see `scripts/launch-local`) and wall-clock logs
with the `[lambdash]` prefix highlight shader compilation or timing diagnostics.

### Path Resolution & Environment Variables

Local shader handles accept shell-style expansions so configs stay portable:

- `~` expands to the current user's home directory (e.g. `~/shaders/demo`).
- `$VAR` / `${VAR}` expand using `std::env::var`; missing variables abort with a descriptive
  error so typos show up immediately.
- Anything containing a `/` is treated literally after expansion, relative to the process working directory unless the path is absolute.
- `local://<pack>` searches `$DATA_DIR` (or `LAMBDASH_DATA_DIR`), then legacy `shaders/` trees under the config/data dirs, and finally `/usr/share/lambdash/shaders/`.

This logic applies across CLI handles (`lambdash $HOME/shaders/demo`), playlist manifests, and
the defaults bootstrap. Run `lambdash defaults where` to inspect which directories are currently
active.

## Release Notes

See `CHANGELOG.md` for released versions and upgrade highlights.
