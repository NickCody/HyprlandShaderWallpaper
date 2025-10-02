# Lambda Shade (lambdash)

Lambda Shade (code-named Lambda Shade) is a Rust-based wallpaper engine
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

By default Lambda Shade assumes ShaderToy-style gamma output: it creates a non-sRGB
swapchain and binds textures without automatic colour conversion, matching the
WebGL defaults used on shadertoy.com. You can override this at several levels:

- **CLI:** `--color-space {auto|gamma|linear}`. `auto` behaves like ShaderToy
  (`gamma`), while `linear` requests sRGB swapchains/textures so physically based
  shaders can output linear light.
- **Manifest:** Local packs may set `color_space = "gamma"` (or `"linear"`) in
  `shader.toml`. CLI overrides manifest; manifest overrides the default.
- **Playlists:** Multi-playlist runs inherit the same hierarchy—global CLI flag
  dominates, otherwise each pack’s manifest decides the swapchain format.

When switching between modes Lambda Shade rebuilds GPU resources so playlists can mix
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

Prefer a scripted setup? Use the curl-friendly installer, which clones the
repository, runs `cargo install`, and seeds bundled shaders/playlists into your
user directories without requiring root:

```bash
bash -c "$(curl -fsSL https://raw.githubusercontent.com/NickCody/HyprlandShaderWallpaper/main/scripts/install.sh)"
```

By default the script copies defaults into `~/.local/share/lambdash` and runs
`lambdash defaults sync`. Pass `--share-dir` to override the target directory or
`--system` (with sudo) to install into `/usr/local` and `/usr/share/lambdash`.
Additional flags help with constrained environments—`--skip-build` reuses an
existing binary, and `--offline` forwards Cargo's offline mode when crates are
already cached. All installer options (including `--prefix`, `--ref`, and
`--no-sync`) are documented via `bash scripts/install.sh --help`.
Ensure `cargo`, `git`, and `tar` are available before running the script.

## Defaults, Directories, and CLI Helpers

Lambda Shade follows the XDG base directory spec and records paths in `state.toml` under
`$XDG_CONFIG_HOME/lambdash` (default `~/.config/lambdash`). The core locations are:

- Config: `~/.config/lambdash/`
- Data: `~/.local/share/lambdash/`
- Cache: `~/.cache/lambdash/`
- System defaults: `/usr/share/lambdash/` (overridable via `LAMBDASH_SHARE_DIR`)

Set `LAMBDASH_CONFIG_DIR`, `LAMBDASH_DATA_DIR`, and `LAMBDASH_CACHE_DIR` to relocate
any directory. CLI flags always win over environment variables.

Bundled shader packs and sample playlists live under the share directory. The
installer script copies everything from `local-shaders/` and `multi/` into your
chosen share root (defaulting to `~/.local/share/lambdash`) and writes a
`VERSION` stamp capturing the Git reference. Lambda Shade never creates the share
tree on its own; instead it syncs whatever is already present into user space.

Use `lambdash defaults` to manage those copies:

- `lambdash defaults sync` copies missing packs/playlists into the user data tree.
  Add `--dry-run` to see pending changes without touching disk.
- `lambdash defaults list` reports which bundled assets are installed locally.
- `lambdash defaults where` prints the resolved config/data/cache/share paths.

To bootstrap a new environment without launching the daemon, run
`lambdash defaults sync --dry-run` (to inspect) and `lambdash defaults sync` (to
install). The daemon also accepts `--init-defaults` for a one-shot sync and exit.

### Still Frames & Exports

- `--still` renders one frame and pauses the scheduler, keeping the surface alive for previews.
- `--still-time <seconds|auto>` selects the timestamp for still/export captures (auto currently falls back to `0`).
- `--still-random-seed <seed>` forwards a deterministic seed to shaders that use randomness.
- `--still-export <path.png>` grabs the next frame to disk; the renderer forces window mode for capture.
- `--still-exit <true|false>` controls whether Lambda Shade exits after finishing a still export (defaults to `true`).
  When exporting, Lambda Shade hides the preview window unless `--window` is supplied, so headless
  captures complete without flashing a surface.

### Quality & Adaptive Controls

- `--render-scale <0.25-1.0>` supersamples or downsamples before presenting to the compositor.
- `--fill-method stretch|center:WxH|tile[:XxY]` remaps shader coordinates, enabling letterboxing or tiled repeats.
- `--fps-adaptive` enables cadence throttling when the compositor hides the surface; pair with `--max-fps-occluded <fps>` to cap the hidden refresh rate.
- Existing `--fps <value>` remains the steady-state cap while the surface is visible.

## Packaging Guidance

Downstream packages and automation should mirror the installer’s behaviour:

- Invoke `scripts/install.sh --skip-build --share-dir <dest>` during packaging to
  materialise `local-shaders/` and `multi/` into a staging directory. The script
  writes `VERSION`, so include that file in the package for upgrade detection.
- When producing system packages (`.deb`, `.rpm`, etc.), call the script with
  `--system` or provide explicit `--prefix`/`--share-dir` flags that match your
  filesystem layout. Running `lambdash defaults sync --dry-run` in post-install
  hooks gives users visibility into which assets were installed.
- Avoid running as root unless shipping a system-wide package. For user-focused
  bundles (AppImage, Flatpak, etc.) set `LAMBDASH_SHARE_DIR` to a writable path
  and run the installer in `--skip-build` mode after the binary is staged.
- CI should execute `cargo test -p lambdash` to cover the installer integration
  test (`install_script_copies_defaults`) and ensure future changes keep the
  script functional.

## Workspace Layout

- `crates/lambdash`: Daemon entry point and CLI that orchestrates rendering.
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
cargo run -p lambdash -- --multi multi/workspaces.toml
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
- Relative paths first check the process working directory, then the resolved XDG config/data
  directories, and finally `/usr/share/lambdash`.

This logic applies across CLI handles (`lambdash $HOME/shaders/demo`), playlist manifests, and
the defaults bootstrap. Run `lambdash defaults where` to inspect which directories are currently
active.

## Release Notes

See `CHANGELOG.md` for released versions and upgrade highlights.
