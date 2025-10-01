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

By default Hyshadew assumes ShaderToy-style gamma output: it creates a non-sRGB
swapchain and binds textures without automatic colour conversion, matching the
WebGL defaults used on shadertoy.com. You can override this at several levels:

- **CLI:** `--color-space {auto|gamma|linear}`. `auto` behaves like ShaderToy
  (`gamma`), while `linear` requests sRGB swapchains/textures so physically based
  shaders can output linear light.
- **Manifest:** Local packs may set `color_space = "gamma"` (or `"linear"`) in
  `shader.toml`. CLI overrides manifest; manifest overrides the default.
- **Playlists:** Multi-playlist runs inherit the same hierarchy—global CLI flag
  dominates, otherwise each pack’s manifest decides the swapchain format.

When switching between modes Hyshadew rebuilds GPU resources so playlists can mix
gamma-authored and linear-authored content without restarting the daemon.

## Install from Git

Install the binary straight from this repository without cloning it:

```bash
cargo install \
  --git https://github.com/NickCody/HyprlandShaderWallpaper \
  --tag v0.9.1 \
  --locked \
  hyshadew
```

During active development you can substitute `--branch main` to follow the latest
changes. Add `--force` to reinstall after pulling a new tag.

If you already have the repository checked out, the same command works locally:

```bash
cargo install --path crates/hyshadew --locked --force hyshadew
```

The installed `hyshadew` binary accepts the same CLI flags documented below, so you
can run `hyshadew --window --shadertoy https://www.shadertoy.com/view/3dXyWj` from
any directory.

## Installer Script

Prefer a scripted setup? Use the curl-friendly installer, which clones the
repository, runs `cargo install`, and seeds bundled shaders/playlists into your
user directories without requiring root:

```bash
bash -c "$(curl -fsSL https://raw.githubusercontent.com/NickCody/HyprlandShaderWallpaper/main/scripts/install.sh)"
```

By default the script copies defaults into `~/.local/share/hyshadew` and runs
`hyshadew defaults sync`. Pass `--share-dir` to override the target directory or
`--system` (with sudo) to install into `/usr/local` and `/usr/share/hyshadew`.
Additional flags help with constrained environments—`--skip-build` reuses an
existing binary, and `--offline` forwards Cargo's offline mode when crates are
already cached. All installer options (including `--prefix`, `--ref`, and
`--no-sync`) are documented via `bash scripts/install.sh --help`.
Ensure `cargo`, `git`, and `tar` are available before running the script.

## Defaults, Directories, and CLI Helpers

Hyshadew follows the XDG base directory spec and records paths in `state.toml` under
`$XDG_CONFIG_HOME/hyshadew` (default `~/.config/hyshadew`). The core locations are:

- Config: `~/.config/hyshadew/`
- Data: `~/.local/share/hyshadew/`
- Cache: `~/.cache/hyshadew/`
- System defaults: `/usr/share/hyshadew/` (overridable via `HYSHADEW_SHARE_DIR`)

Set `HYSHADEW_CONFIG_DIR`, `HYSHADEW_DATA_DIR`, and `HYSHADEW_CACHE_DIR` to relocate
any directory. CLI flags always win over environment variables.

Bundled shader packs and sample playlists live under the share directory. The
installer script copies everything from `local-shaders/` and `multi/` into your
chosen share root (defaulting to `~/.local/share/hyshadew`) and writes a
`VERSION` stamp capturing the Git reference. Hyshadew never creates the share
tree on its own; instead it syncs whatever is already present into user space.

Use `hyshadew defaults` to manage those copies:

- `hyshadew defaults sync` copies missing packs/playlists into the user data tree.
  Add `--dry-run` to see pending changes without touching disk.
- `hyshadew defaults list` reports which bundled assets are installed locally.
- `hyshadew defaults where` prints the resolved config/data/cache/share paths.

To bootstrap a new environment without launching the daemon, run
`hyshadew defaults sync --dry-run` (to inspect) and `hyshadew defaults sync` (to
install). The daemon also accepts `--init-defaults` for a one-shot sync and exit.

## Packaging Guidance

Downstream packages and automation should mirror the installer’s behaviour:

- Invoke `scripts/install.sh --skip-build --share-dir <dest>` during packaging to
  materialise `local-shaders/` and `multi/` into a staging directory. The script
  writes `VERSION`, so include that file in the package for upgrade detection.
- When producing system packages (`.deb`, `.rpm`, etc.), call the script with
  `--system` or provide explicit `--prefix`/`--share-dir` flags that match your
  filesystem layout. Running `hyshadew defaults sync --dry-run` in post-install
  hooks gives users visibility into which assets were installed.
- Avoid running as root unless shipping a system-wide package. For user-focused
  bundles (AppImage, Flatpak, etc.) set `HYSHADEW_SHARE_DIR` to a writable path
  and run the installer in `--skip-build` mode after the binary is staged.
- CI should execute `cargo test -p hyshadew` to cover the installer integration
  test (`install_script_copies_defaults`) and ensure future changes keep the
  script functional.

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

### Path Resolution & Environment Variables

Local shader handles accept shell-style expansions so configs stay portable:

- `~` expands to the current user's home directory (e.g. `~/shaders/demo`).
- `$VAR` / `${VAR}` expand using `std::env::var`; missing variables abort with a descriptive
  error so typos show up immediately.
- Relative paths first check the process working directory, then the resolved XDG config/data
  directories, and finally `/usr/share/hyshadew`.

This logic applies across CLI handles (`hyshadew $HOME/shaders/demo`), playlist manifests, and
the defaults bootstrap. Run `hyshadew defaults where` to inspect which directories are currently
active.

## Release Notes

See `CHANGELOG.md` for released versions and upgrade highlights.
