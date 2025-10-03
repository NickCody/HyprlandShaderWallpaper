# Lambda Shader Overview

Lambda Shader (lambdash) is a Rust-based wallpaper engine focused on Wayland compositors. It renders GPU shaders as live backgrounds while offering deep compatibility with ShaderToy content and user-supplied shader packs.

## Workspace Layout

- `crates/lambdash`: Wayland-facing daemon. Handles configuration, CLI flags, ShaderToy client setup, and orchestrates rendering.
- `crates/renderer`: Rendering abstraction that will host `wgpu`/OpenGL logic and manage frame uniforms for ShaderToy-style shaders.
- `crates/shadertoy`: Integration layer that fetches ShaderToy metadata, caches shaders/assets locally, validates manifest layouts, and unifies local/remote shader sources.
- `local-shaders/`: User drop-in shader packs mirroring ShaderToy renderpass structure (GLSL sources, textures, cubemaps, audio, manifest).

## Key Features

- **ShaderToy API support**: `ShadertoyClient` downloads shader JSON, GLSL code, and assets, converting them into validated `shader.toml` manifests ready for the renderer.
- **Local pack compatibility**: Users can place shader directories in `local-shaders/`; the loader validates channel bindings, textures, cubemaps, and audio resources.
- **Unified repository**: `ShaderRepository` resolves local packs or cached ShaderToy shaders, refreshing remote caches when API credentials are supplied.
- **Path and defaults**: Local handles honour `~` and shell-style `$VAR`/`${VAR}` expansions, then search the working directory, XDG config/data roots, and `/usr/share/lambdash`. Missing variables fail fast so misconfigurations surface immediately. `lambdash defaults sync|list|where` manage copies of bundled shader packs from the system share tree into user space.
- **Installer script**: `scripts/install.sh` (curlable via GitHub) performs a user-mode install by default, copying bundled shaders into `~/.local/share/lambdash` and running `lambdash defaults sync`. Use `--system` for `/usr/local` + `/usr/share/lambdash`, or pass `--share-dir`/`--prefix` to target custom locations.
- **CLI-driven daemon**: `lambdash` accepts handles like `shadertoy://ID` or local paths, supports cache-only/refresh switches, a `--shadertoy <url>` convenience flag, and `--window` testing mode.

## Next Steps

1. Harden the Wayland wallpaper path (recover from surface loss, validate config edge cases).
2. Bind ShaderToy uniform expectations (`iTime`, `iMouse`, channels) inside the renderer.
3. Expand asset handling for advanced channel types (webcam/keyboard), and provide richer diagnostics/UI for shader selection.

## Renderer Animation Debugging (Sept 26, 2025)

- Verified CPU-side uniforms (`iTime`, `iFrame`, `iMouse`) advance each frame and log once per second.
- Rendering loop now requests redraws on `AboutToWait` with `ControlFlow::Wait` to match vblank cadence.
- Fragment wrapper uses hardware `gl_FragCoord`, flips Y once, and maps ShaderToy uniforms through a `ShaderParams` UBO with macros.
- Strips legacy `uniform` declarations (including `iTime`, `iChannel*`) from fetched GLSL before injecting wrapper code.
- Debug mode currently replaces shader output with a full-screen pulse driven by `iTime`; wrapped GLSL dumps to `/tmp/lambdash_wrapped.frag` for inspection each compile.

## Multi-Playlist Runtime Notes (Sept 26, 2025)

- Stage 4 playlist work is complete: workspace switches now preempt in-flight transitions, reusing
  `workspace_switch_crossfade` (set it to `0` for hard cuts).
- The playlist engine emits info-level telemetry (`registered new playlist target`, `retargeted playlist`,
  and `swapping shader`) detailing selectors, crossfade durations, warmup, and FPS overrides. Additional
  diagnostics surface at `debug` when Hyprland snapshots fail or shader assets are reused.
- Wall-clock diagnostics now use the `[lambdash]` prefix instead of `[lambdash]`; tracing output goes
  through `scripts/launch-local`.
- Tests covering workspace crossfades and failure handling live in `crates/lambdash/src/multi.rs` (`workspace_switch_applies_crossfade_override`
  and `engine_skips_missing_items_and_advances`).
- Sample playlists are in `local-shaders/default.toml` and `local-shaders/workspaces.toml`.

### To-Do for Next Agent

- Run `cargo run -p lambdash -- --shadertoy https://www.shadertoy.com/view/3dXyWj --window`, then inspect `/tmp/lambdash_wrapped.frag` to ensure no leftover `uniform iTime`/`iChannel*` lines remain and macros look correct.
- If the pulse still fails to animate, try feeding the wrapped GLSL through naga to WGSL (or compile via shaderc/SPIR-V) to rule out wgpu's GLSL frontend quirks.
- Once animation is confirmed, revert the pulse override to blend with `mainImage`, then restore the original shader output.

### Defaults & Directory Primer

- Share tree (`/usr/share/lambdash` by default) is supplied by packaging; lambdash only mirrors its contents into user directories. If the directory is missing, `defaults::sync_defaults` logs a debug skip and does nothing.
- User directories: config `~/.config/lambdash`, data `~/.local/share/lambdash`, cache `~/.cache/lambdash`. Override with `LAMBDASH_CONFIG_DIR`, `LAMBDASH_DATA_DIR`, `LAMBDASH_CACHE_DIR`, `LAMBDASH_SHARE_DIR`.
- Inspect paths with `lambdash defaults where`. Run `lambdash defaults sync --dry-run` before making changes; `--init-defaults` performs the same sync during daemon startup.
- The installer script defaults to user-space share directories; encourage contributors to avoid root unless packaging for system-wide deployment.
- For packaging, reuse `scripts/install.sh --skip-build --share-dir <dest>` to stage shader assets (shader packs + playlists) and include the generated `VERSION` file; CI runs `cargo test -p lambdash` to exercise the script via `install_script_copies_defaults`.
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
