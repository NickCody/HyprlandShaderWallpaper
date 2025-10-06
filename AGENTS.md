# WallShader Overview

WallShader (wallshader) is a Rust-based wallpaper engine focused on Wayland compositors. It renders GPU shaders as live backgrounds while offering deep compatibility with ShaderToy content and user-supplied shader packs.

## Agent Instructions

- Run `just validate` to check formatting, linting, and run tests before finishing work.
- When making changes to a module, always keep t he top-of-file comments and `mod` declarations in sync.

## Workspace Layout

- `crates/wallshader`: Wayland-facing daemon. Handles configuration, CLI flags, ShaderToy client setup, and orchestrates rendering.
- `crates/renderer`: Rendering abstraction that will host `wgpu`/OpenGL logic and manage frame uniforms for ShaderToy-style shaders.
- `crates/shadertoy`: Integration layer that fetches ShaderToy metadata, caches shaders/assets locally, validates manifest layouts, and unifies local/remote shader sources.
- `shaders/`: User drop-in shader packs mirroring ShaderToy renderpass structure (GLSL sources, textures, cubemaps, audio, manifest).

## Key Features

- **ShaderToy API support**: `ShadertoyClient` downloads shader JSON, GLSL code, and assets, converting them into validated `shader.toml` manifests ready for the renderer.
- **Shader pack compatibility**: Users can place shader directories in `shaders/`; the loader validates channel bindings, textures, cubemaps, and audio resources.
- **Unified repository**: `ShaderRepository` resolves shader packs or cached ShaderToy shaders, refreshing remote caches when API credentials are supplied.
- **Path handling**: `shader://<pack>` and `playlist://<name>` handles honour `~` and shell-style `$VAR`/`${VAR}` expansions, then search the data dir, legacy `shaders/` trees, and `/usr/share/wallshader`. Anything containing a `/` is interpreted literally after expansion. Missing variables fail fast so misconfigurations surface immediately.
- **Installer script**: `scripts/install.sh` (curlable via GitHub) performs a user-mode install by default, copying bundled shaders into `~/.local/share/wallshader/` (packs become subdirectories, playlists flatten to top-level `.toml`). Use `--system` for `/usr/local` + `/usr/share/wallshader`, or pass `--data-dir`/`--prefix` to target custom locations.
- **CLI-driven daemon**: `wallshader` accepts handles like `shadertoy://ID` or local paths, supports cache-only/refresh switches, a `--shadertoy <url>` convenience flag, and `--window` testing mode.

## Next Steps

1. Harden the Wayland wallpaper path (recover from surface loss, validate config edge cases).
2. Bind ShaderToy uniform expectations (`iTime`, `iMouse`, channels) inside the renderer.
3. Expand asset handling for advanced channel types (webcam/keyboard), and provide richer diagnostics/UI for shader selection.

## Renderer Animation Debugging (Sept 26, 2025)

- Verified CPU-side uniforms (`iTime`, `iFrame`, `iMouse`) advance each frame and log once per second.
- Rendering loop now requests redraws on `AboutToWait` with `ControlFlow::Wait` to match vblank cadence.
- Fragment wrapper uses hardware `gl_FragCoord`, flips Y once, and maps ShaderToy uniforms through a `ShaderParams` UBO with macros.
- Strips legacy `uniform` declarations (including `iTime`, `iChannel*`) from fetched GLSL before injecting wrapper code.
- Debug mode currently replaces shader output with a full-screen pulse driven by `iTime`; wrapped GLSL dumps to `/tmp/wallshader_wrapped.frag` for inspection each compile.

## Multi-Playlist Runtime Notes (Sept 26, 2025)

- Stage 4 playlist work is complete: workspace switches now preempt in-flight transitions, reusing
  `workspace_switch_crossfade` (set it to `0` for hard cuts).
- The playlist engine emits info-level telemetry (`registered new playlist target`, `retargeted playlist`,
  and `swapping shader`) detailing selectors, crossfade durations, warmup, and FPS overrides. Additional
  diagnostics surface at `debug` when Hyprland snapshots fail or shader assets are reused.
- Wall-clock diagnostics now use the `[wallshader]` prefix instead of `[wallshader]`; tracing output goes
  through `scripts/launch-local`.
- Tests covering workspace crossfades and failure handling live in `crates/wallshader/src/multi.rs` (`workspace_switch_applies_crossfade_override`
  and `engine_skips_missing_items_and_advances`).
- Sample playlists live in `shaders/playlists/` within the repo and install as `$DATA_DIR/*.toml`.

### To-Do for Next Agent

- Run `cargo run -p wallshader -- --shadertoy https://www.shadertoy.com/view/3dXyWj --window`, then inspect `/tmp/wallshader_wrapped.frag` to ensure no leftover `uniform iTime`/`iChannel*` lines remain and macros look correct.
- If the pulse still fails to animate, try feeding the wrapped GLSL through naga to WGSL (or compile via shaderc/SPIR-V) to rule out wgpu's GLSL frontend quirks.
- Once animation is confirmed, revert the pulse override to blend with `mainImage`, then restore the original shader output.

### Directory Primer

- User directories: config `~/.config/wallshader`, data `~/.local/share/wallshader`, cache `~/.cache/wallshader`. Override with `WALLSHADER_CONFIG_DIR`, `WALLSHADER_DATA_DIR`, `WALLSHADER_CACHE_DIR`, `WALLSHADER_SHARE_DIR`.
- **Resolution order:** CONFIG_DIR > DATA_DIR > SHARE_DIR > dev-root. This ensures user customizations in CONFIG_DIR always take precedence.
- **To customize shaders:** Copy from DATA_DIR or SHARE_DIR to CONFIG_DIR, then edit. WallShader will prefer the CONFIG_DIR version.
- **AppImage first-run:** On startup, bundled assets from SHARE_DIR (`$APPDIR/opt/wallshader`) are automatically extracted to DATA_DIR once per version, always overwriting. CONFIG_DIR is never touched by the extractor.
- Inspect paths with `wallshader defaults where` to confirm the active layout.
- The installer script now populates the data directory directly (`$DATA_DIR/shaders/<pack>` directories and `$DATA_DIR/playlists/*.toml` files); encourage contributors to avoid root unless packaging for system-wide deployment.
- For packaging, reuse `scripts/install.sh --skip-build --data-dir <dest>` to stage shader assets (shader packs + playlists).
- Expect env interpolation failures (`$VAR`) to abort load; log output will pinpoint the missing variable.

### TODO: Wallpaper Power Optimisation

- Goal: avoid rendering when the wallpaper is fully obscured or otherwise not visible to the user.
- Hook ideas:
  * Listen for `wl_surface::frame` starvation and stop scheduling new frames until the compositor wakes the surface again (Hyprland already throttles us when covered).
  * Investigate Hyprland's idle/active surface signals or `hyprland-idle-inhibit` protocol for explicit visibility hints.
  * Consider `presentation-time` or `wp_fractional_scale` events to detect when the surface is off-screen or scaled to zero.
- Implementation sketch:
  1. Add tracing around frame callbacks to confirm when throttling kicks in.
  2. Introduce a state flag (e.g. `is_visible`) and short-circuit our own `schedule_frame` when the compositor indicates we are hidden. Resume once a configure/frame arrives.
  3. Surface the behaviour via CLI/manifest so users can choose between "always animate" and "pause when hidden" modes.
- Deliverables: documentation entry in `AGENTS.md`, optional CLI toggle, and telemetry in logs so future agents can verify throttling behaviour quickly.

### TODO: Fill Method Options

- Goal: expose `--fill-method {stretch|center|tile}` so users can control how shaders map onto the wallpaper surface.
- Approach: implement the policies inside the GLSL wrapper rather than relying on the compositor.
  * `stretch` (default): current behaviour; map `fragCoord` directly to the surface size.
  * `center`: render into a user-specified content resolution and letterbox the rest; early-out when pixels fall outside the centered region.
  * `tile`: wrap coordinates with `mod` to repeat the shader across the surface.
- Plumbing: add enum to `RendererConfig`, propagate from CLI, emit per-mode helper macros/uniforms when wrapping the shader.
- Open questions: do we allow custom content resolution per shader (e.g. 1080p rendered on 4K), and should tile/center pick up `--size` overrides?

## GPU Resource Friendliness (Oct 5, 2025)

**Problem:** WallShader was causing Firefox to freeze when rendering ShaderToy WebGL content, due to aggressive GPU resource claims.

**Changes made:**
1. **Power Preference**: Switched from `HighPerformance` to `LowPower` in adapter request (gpu.rs:178)
   - Wallpaper rendering doesn't need maximum GPU priority
   - Yields GPU resources to interactive applications like browsers
   
2. **Memory Hints**: Changed from `Performance` to `MemoryUsage` in device creation (gpu.rs:317)
   - Reduces GPU memory pressure
   - Allows other applications to allocate resources more easily
   
3. **Frame Latency**: Increased from 1 to 2 frames (gpu.rs:351)
   - Wallpaper doesn't require minimal latency
   - Gives driver more flexibility to schedule work alongside interactive apps
   - Reduces GPU contention

**Impact:** These changes make WallShader a better "background citizen" that won't starve foreground GPU applications. The visual quality and animation smoothness should remain unchanged since wallpapers don't need aggressive performance optimization.

### Command-Line Control (Oct 5, 2025)

Added three new CLI flags to allow users to control GPU resource usage:

- `--gpu-power <MODE>`: GPU power preference
  - `low` (default): Friendly to other applications, yields GPU priority
  - `high`: Maximum performance, aggressive GPU usage
  
- `--gpu-memory <MODE>`: GPU memory allocation priority
  - `balanced` (default): Friendly memory usage, allows other apps to allocate
  - `performance`: Maximum memory allocation priority
  
- `--gpu-latency <FRAMES>`: GPU frame latency (1-3, default 2)
  - Lower values (1) = minimal latency but more GPU contention
  - Higher values (2-3) = better sharing with other applications

**Default behavior:** Friendly settings that won't interfere with other applications.

**High-performance mode example:**
```bash
wallshader --gpu-power high --gpu-memory performance --gpu-latency 1
```

**Implementation:**
- Added types to `renderer/src/types.rs`: `GpuPowerPreference`, `GpuMemoryMode`
- Added CLI types and parsers to `wallshader/src/cli.rs`
- Extended `RendererConfig` with GPU settings
- Updated `GpuState::new()` to accept and use GPU configuration
- Conversion functions in `run.rs` to bridge CLI and renderer types
