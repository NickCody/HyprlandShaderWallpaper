# WallShader

WallShader is a Rust-based wallpaper engine for Wayland compositors. It renders ShaderToy-compatible GPU shaders as live backgrounds and supports both remote ShaderToy content and local shader packs. It can run in windowed mode for testing or as a daemon that manages multiple outputs and workspaces, with optional playlist support for rotating wallpapers. 

## AppImage Installation (Recommended)

### Prerequisites for AppImage

* A Wayland compositor (Hyprland, Sway, KDE Wayland, GNOME Wayland, etc.)
* GPU drivers with Vulkan or OpenGL support (Mesa works great)

All other dependencies are bundled in the AppImage.

### Download and Run

1. Download the latest AppImage from the [Releases page](https://github.com/NickCody/WallShader/releases)
2. Make it executable and run:

```bash
chmod +x WallShader-x86_64.AppImage
./WallShader-x86_64.AppImage --window --shadertoy https://www.shadertoy.com/view/3dXyWj
```

Optional: move it to your PATH for convenience:

```bash
mkdir -p ~/.local/bin
mv WallShader-x86_64.AppImage ~/.local/bin/wallshader
wallshader --help
```

## Building from Source

### Build Prerequisites

All platforms need:

* Rust toolchain (`rustup`) + `rustfmt` and `clippy`
* C/C++ build tools (`gcc`/`cc`, `g++`, `make`, binutils)
* **CMake** (and optionally `ninja` for faster builds)
* Wayland dev headers (`wayland-protocols`, `wayland`/`wayland-client`, `libxkbcommon`, `pkg-config`)
* GPU drivers with Vulkan or OpenGL (Mesa works great)

#### Install Rust (all distros)

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"
rustup component add rustfmt clippy
```

#### Ubuntu / Debian

```bash
sudo apt update
sudo apt install -y \
  build-essential pkg-config cmake ninja-build \
  libwayland-dev wayland-protocols libxkbcommon-dev \
  libegl1-mesa-dev libgles2-mesa-dev \
  mesa-vulkan-drivers vulkan-tools libvulkan-dev
```

#### Fedora

```bash
sudo dnf install -y \
  gcc gcc-c++ make glibc-devel binutils cmake ninja-build \
  pkgconf-pkg-config wayland-devel wayland-protocols-devel libxkbcommon-devel \
  mesa-libEGL-devel mesa-libGLES-devel \
  mesa-vulkan-drivers vulkan-headers vulkan-loader-devel vulkan-tools
```

#### Arch / Manjaro

```bash
sudo pacman -Syu --needed \
  base-devel pkgconf cmake ninja \
  wayland wayland-protocols libxkbcommon \
  mesa \
  vulkan-icd-loader vulkan-tools
```

#### openSUSE (Tumbleweed / Leap)

```bash
sudo zypper install -t pattern devel_C_C++
sudo zypper install -y \
  gcc gcc-c++ make binutils cmake ninja \
  pkg-config wayland-devel wayland-protocols-devel libxkbcommon-devel \
  Mesa-libEGL-devel Mesa-libGLESv2-devel \
  vulkan-tools vulkan-headers libvulkan1
```

#### Alpine

```bash
sudo apk add --update \
  build-base binutils pkgconf cmake ninja \
  wayland-dev wayland-protocols libxkbcommon-dev \
  mesa-egl-dev mesa-gles-dev \
  vulkan-loader vulkan-headers vulkan-tools
```

#### NixOS (ad-hoc dev shell)

```bash
nix-shell -p \
  gcc pkg-config cmake ninja \
  wayland wayland-protocols libxkbcommon \
  mesa mesa_drivers \
  vulkan-loader vulkan-tools
# Then install Rust via rustup inside the shell.
```

**Tip:** If your Vulkan driver is distro-specific, ensure the correct ICD is present (e.g., `mesa-vulkan-drivers` on Debian/Ubuntu/Fedora, `vulkan-icd-loader` on Arch).

### Installation Methods

#### One-liner Installer Script

Prefer a scripted setup? Use the curl-friendly installer that clones the repository, runs `cargo install`, and copies bundled shader packs into your wallpaper data directory without requiring root:

```bash
bash -c "$(curl -fsSL https://raw.githubusercontent.com/NickCody/WallShader/main/scripts/install.sh)"
```

By default this grabs `main` from GitHub, overwriting `~/.local/share/wallshader/shaders` with the bundled shader packs and reinstalling the binary. Use `--data-dir` to pick a different destination or `--system` (with sudo) to install under `/usr/share/wallshader`. Additional flags help with constrained environments—`--skip-build` reuses an existing binary, and `--offline` forwards Cargo's offline mode. All installer options (including `--prefix` and `--ref`) are documented via `bash scripts/install.sh --help`. Ensure `cargo`, `git`, and `tar` are available before running.

Working from a local checkout? Run the installer directly:

```bash
scripts/install.sh --source .
```

Pass `--data-dir ~/.local/share/wallshader` (or `--system`) to target a specific location. Re-run whenever you want to refresh shader packs while iterating.

#### Clone and Build with Just

Install `just` via your package manager or with `cargo install just`, then:

```bash
git clone https://github.com/NickCody/WallShader.git
cd WallShader
just setup
just run-demo
```

The repository includes a `justfile` with common workflows:

`just setup` will install the required toolchain components and pre-fetch crates so the first build runs quickly. The demo uses a ShaderToy URL; provide your own API key via `--shadertoy-api-key` or the `SHADERTOY_API_KEY` environment variable if the shader is not cached locally.

```
just check      # fmt + clippy over the workspace
just run-demo   # windowed ShaderToy demo
just run-playlist  # playlist sampler using workspaces.toml
```

Run `just --list` to discover additional recipes.

#### Cargo Install from Repository

Install the binary straight from the repository without cloning it:

```bash
cargo install \
  --git https://github.com/NickCody/WallShader \
  --tag v0.9.1 \
  --locked \
  wallshader
```

During active development you can substitute `--branch main` to follow the latest changes. Add `--force` to reinstall after pulling a new tag.

If you already have the repository checked out, the same command works locally:

```bash
cargo install --path crates/wallshader --locked --force wallshader
```

The installed `wallshader` binary accepts the same CLI flags documented below, so you can run `wallshader --window --shadertoy https://www.shadertoy.com/view/3dXyWj` from any directory.

## Features

### ShaderToy Integration

WallShader can fetch and cache public ShaderToy shaders using the `--shadertoy <url>` flag. Supply your API key via `--shadertoy-api-key <key>` or the `SHADERTOY_API_KEY` environment variable. The installer bundles a few popular shaders under `shaders/shadertoy/` so you can try them without an API key.

### Color Space & Gamma Handling

By default WallShader assumes ShaderToy-style gamma output: it creates a non-sRGB swapchain and binds textures without automatic colour conversion, matching the WebGL defaults used on shadertoy.com. You can override this at several levels:

- **CLI:** `--color-space {auto|gamma|linear}`. `auto` behaves like ShaderToy
  (`gamma`), while `linear` requests sRGB swapchains/textures so physically based
  shaders can output linear light.
- **Manifest:** Shader packs may set `color_space = "gamma"` (or `"linear"`) in
  `shader.toml`. CLI overrides manifest; manifest overrides the default.
- **Playlists:** Multi-playlist runs inherit the same hierarchy—global CLI flag
  dominates, otherwise each pack’s manifest decides the swapchain format.

When switching between modes WallShader rebuilds GPU resources so playlists can mix gamma-authored and linear-authored content without restarting the daemon.

### Playlists

Enable playlist mode with `--playlist <file>` to drive different shaders per workspace or output. The configuration format is documented in `SpecMulti.md`, and sample playlists live under `shaders/playlists/` in the repo (e.g. `workspaces.toml`) and are copied to `$DATA_DIR/*.toml` by the installer. A quick way to experiment is:

```
cargo run -p wallshader -- --playlist workspaces.toml
```

Key behaviours:

- Each output resolves to a playlist via workspace or output selectors.
- Workspace changes interrupt any active transition and start a new crossfade using
  `workspace_switch_crossfade` (set to `0` for hard cuts).
- Playlist items can override FPS caps, antialiasing, and opt-in to one-time refreshes.
- Playlist entries now support `mode = "still"` (optionally `still_time = "2s"`) for photo-style slides that render once.

Runtime telemetry is emitted via `tracing` (see `scripts/launch-local`) and wall-clock logs with the `[wallshader]` prefix highlight shader compilation or timing diagnostics.

### Directories and CLI Helpers

WallShader follows the XDG base directory spec. The core locations are:

- Config: `~/.config/wallshader/` — **User customizations and overrides**
- Data: `~/.local/share/wallshader/` — **Installed shader packs and playlists**
- Cache: `~/.cache/wallshader/` — **ShaderToy cache and temporary files**
- System defaults: `/usr/share/wallshader/` — **Bundled assets (AppImage, system packages)**

Set `WALLSHADER_CONFIG_DIR`, `WALLSHADER_DATA_DIR`, `WALLSHADER_CACHE_DIR`, and `WALLSHADER_SHARE_DIR` to relocate any directory. CLI flags always win over environment variables.

#### Directory Resolution Order

Shaders and playlists are resolved in priority order: **CONFIG > DATA > SHARE > dev-root**

1. **CONFIG_DIR** (`~/.config/wallshader/`) — Your personal modifications take precedence
2. **DATA_DIR** (`~/.local/share/wallshader/`) — Installer-managed shader packs
3. **SHARE_DIR** (`/usr/share/wallshader/`) — System-wide or AppImage-bundled defaults
4. **dev-root** — Detected workspace root during development

**To customize a shader:** Copy it from DATA_DIR or SHARE_DIR to CONFIG_DIR, then edit your copy. WallShader will always prefer your CONFIG_DIR version.

**AppImage behavior:** On first run, bundled shaders and playlists are automatically extracted from the AppImage's SHARE_DIR to your DATA_DIR. This happens once per version and always overwrites DATA_DIR to provide updates. Your CONFIG_DIR customizations are never touched.

Bundled shader packs and sample playlists live under `shaders/` and `playlists/` in the repository. The installer mirrors those directories to your data location, and you can rerun it any time you want to refresh the packs while developing.

Run `wallshader defaults where` to print the resolved config/data/cache/share paths if you need to double-check the environment.

### Path Resolution & Environment Variables

Local shader handles accept shell-style expansions so configs stay portable:

- `~` expands to the current user's home directory (e.g. `~/shaders/demo`).
- `$VAR` / `${VAR}` expand using `std::env::var`; missing variables abort with a descriptive
  error so typos show up immediately.
- Anything containing a `/` is treated literally after expansion, relative to the process working directory unless the path is absolute.
- `shader://<pack>` searches CONFIG_DIR, then DATA_DIR, then legacy `shaders/` trees, and finally SHARE_DIR.

This logic applies across CLI handles (`wallshader $HOME/shaders/demo`), playlist manifests, and the defaults bootstrap. Run `wallshader defaults where` to inspect which directories are currently active.

### Still Frames & Exports

- `--still` renders one frame and pauses the scheduler, keeping the surface alive for previews.
- `--still-time <seconds|auto>` selects the timestamp for still/export captures (auto currently falls back to `0`).
- `--still-export <path.png>` grabs the next frame to disk; the renderer forces window mode for capture.
- `--still-exit <true|false>` controls whether WallShader exits after finishing a still export (defaults to `true`).
  When exporting, WallShader hides the preview window unless `--window` is supplied, so headless
  captures complete without flashing a surface.

### Quality & Adaptive Controls

- `--render-scale <0.25-1.0>` supersamples or downsamples before presenting to the compositor.
- `--fill-method stretch|center:WxH|tile[:XxY]` remaps shader coordinates, enabling letterboxing or tiled repeats.
- `--fps-adaptive` enables cadence throttling when the compositor hides the surface; pair with `--max-fps-occluded <fps>` to cap the hidden refresh rate.
- Existing `--fps <value>` remains the steady-state cap while the surface is visible.

### Performance & GPU Options

WallShader provides fine-grained control over GPU resource usage and rendering performance. By default, WallShader uses **friendly settings** that prioritize being a good "background citizen" and won't interfere with other GPU-intensive applications like web browsers.

#### Frame Rate Control

**`--fps <number>`**
- **Default**: Unlimited (renders as fast as possible)
- **Valid values**: Any positive number, or `0` for uncapped
- **Example**: `--fps 60` caps rendering at 60 frames per second
- **Use case**: Reduce CPU/GPU usage for static or slow-moving shaders

**`--fps-adaptive`**
- **Default**: Disabled
- **Effect**: Enables dynamic FPS throttling when the wallpaper surface is occluded or minimized
- **Must be combined with**: `--max-fps-occluded`
- **Use case**: Save power when wallpaper isn't visible

**`--max-fps-occluded <number>`**
- **Default**: Not set (requires `--fps-adaptive` to take effect)
- **Valid values**: Any positive number
- **Example**: `--max-fps-occluded 5` limits rendering to 5 FPS when hidden
- **Use case**: Minimal CPU/GPU usage when compositor hides the wallpaper
- **Note**: Only works with `--fps-adaptive` flag

```bash
# Limit to 60 FPS normally, 5 FPS when hidden
wallshader --fps 60 --fps-adaptive --max-fps-occluded 5
```

#### GPU Power Management

**`--gpu-power <mode>`**
- **Default**: `low` (friendly to other applications)
- **Valid values**:
  - `low`, `low-power`, `friendly`, `background` → Low power mode (default)
  - `high`, `high-performance`, `performance`, `max` → Maximum performance mode
- **Example**: `--gpu-power high`
- **What it does**:
  - `low`: Yields GPU priority to interactive applications like browsers, games, and video players. Prevents WallShader from freezing or stuttering other GPU apps.
  - `high`: Requests maximum GPU performance. Use when running complex shaders on a dedicated system or when visual quality is critical.

**`--gpu-memory <mode>`**
- **Default**: `balanced` (friendly memory allocation)
- **Valid values**:
  - `balanced`, `usage`, `normal`, `friendly` → Balanced mode (default)
  - `performance`, `perf`, `max` → Maximum memory allocation
- **Example**: `--gpu-memory performance`
- **What it does**:
  - `balanced`: Reduces GPU memory pressure, allowing other applications to allocate resources more easily
  - `performance`: Requests maximum memory allocation priority for WallShader. May impact other GPU applications.

**`--gpu-latency <frames>`**
- **Default**: `2` (balanced sharing)
- **Valid values**: `1`, `2`, or `3` (automatically clamped to this range)
- **Example**: `--gpu-latency 1`
- **What it does**:
  - `1`: Minimal frame latency for immediate response. Increases GPU contention with other applications.
  - `2`: Balanced latency (default). Gives the GPU driver flexibility to schedule work alongside other apps.
  - `3`: Maximum latency, best for sharing GPU with many applications. Slight input lag may be noticeable on interactive shaders.

#### Performance Presets

**Friendly mode (default)** - Won't interfere with browsers or other GPU apps:
```bash
wallshader shader://simplex
# Equivalent to:
wallshader shader://simplex --gpu-power low --gpu-memory balanced --gpu-latency 2
```

**Maximum performance** - For dedicated wallpaper systems or showcases:
```bash
wallshader shader://simplex --gpu-power high --gpu-memory performance --gpu-latency 1 --fps 120
```

**Battery saver** - Minimal resource usage:
```bash
wallshader shader://simplex --fps 30 --fps-adaptive --max-fps-occluded 1
```

**Balanced high quality** - Good visuals without hogging the GPU:
```bash
wallshader shader://simplex --fps 60 --gpu-power low --render-scale 1.0
```

#### Anti-Aliasing

**`--antialias <mode>`**
- **Default**: `auto` (uses maximum supported sample count)
- **Valid values**:
  - `auto`, `max`, `default` → Automatic maximum MSAA
  - `off`, `none`, `disable`, `disabled`, `0`, `1` → No anti-aliasing
  - `2`, `4`, `8`, `16` → Specific MSAA sample count
- **Example**: `--antialias 4`
- **What it does**: Controls multi-sample anti-aliasing (MSAA) for smoother edges. Higher sample counts look better but use more GPU memory and may reduce framerate on complex shaders.

#### When to Adjust GPU Settings

**Use default (friendly) settings when:**
- Running WallShader alongside web browsers (especially with WebGL/Canvas content)
- Using video conferencing or streaming software
- Playing games while wallpaper is visible
- Running on a laptop or power-constrained system

**Use high-performance settings when:**
- Showcasing complex shaders at events or demos
- Running on a dedicated display/system
- Recording wallpaper footage for videos
- Visual quality is more important than resource sharing
- System has multiple GPUs (dGPU + iGPU)

**Tip**: If you experience stuttering in Firefox, Chrome, or games while WallShader is running, the default `low` power mode should prevent this. If issues persist, try adding `--fps 30` to further reduce GPU load.

## Packaging Guidance

Downstream packages and automation should mirror the installer’s behaviour:

- Invoke `scripts/install.sh --skip-build --data-dir <dest>` during packaging to stage the bundled shader packs (`<dest>/<pack>/`) and playlists (`<dest>/*.toml`). When producing system packages (`.deb`, `.rpm`, etc.), call the script with `--system` or provide explicit `--prefix`/`--data-dir` flags that match your filesystem layout. Avoid running as root unless shipping a system-wide package. For user-focused bundles (AppImage, Flatpak, etc.) set `WALLSHADER_DATA_DIR` to a writable path and run the installer in `--skip-build` mode after the binary is staged. CI should execute `cargo test -p wallshader` to cover the installer integration test (`install_script_copies_defaults`) and ensure future changes keep the script functional.

## Workspace Layout

- `crates/wallshader`: Daemon entry point and CLI that orchestrates rendering.
- `crates/renderer`: Rendering abstraction that manages shader wrapping and frame uniforms.
- `crates/shadertoy`: Integration layer for ShaderToy downloads, caching, and manifest validation.
- `shaders/`: User-provided shader packs mirroring ShaderToy render pass structure.
- `playlists/`: User-provided playlist manifests for multi-shader scheduling.

## Release Notes

See `CHANGELOG.md` for released versions and upgrade highlights.

## Open Issues

- Crossfade doesn't work when you change antialiasing per shader in a playlist.