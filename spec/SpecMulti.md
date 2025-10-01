# Multi-Workspace Playlists (Spec)

Status: Draft for review

This document specifies the multi-workspace playlist system for Hyprland Shader Wallpaper (hyshadew). It introduces a TOML configuration format, scheduling semantics, target (workspace/output) mapping, and runtime behavior in both wallpaper and window modes.

## Goals

- Allow per-workspace and per-output shader selection.
- Support time-based playlists with per-item playback duration.
- Support per-item renderer overrides (fps cap and antialiasing).
- Keep single-shader mode unchanged unless `--multi` is provided.
- Integrate cleanly with caching flags (`--refresh`, `--cache-only`).

## CLI Behavior

- `--multi <path>`: Enables playlist mode and loads the specified multi config TOML. When present, hyshadew runs playlists instead of a single shader.
- `--window`: In playlist mode, uses only the default playlist for preview (ignores all other targets). A default playlist must be configured; otherwise startup fails with a helpful error.
- `--cache-only`: Global. Disables all network fetches. Per-item refresh requests are ignored in this mode.
- `--refresh`: Global. In playlist mode, treated as “refresh once per item this session” (see Caching Semantics). Per-item refresh flags can still be set to opt-in/opt-out at item granularity.
- `defaults where|list|sync`: Diagnostics for resolving multi/shader search paths. Playlist validation should instruct operators to run `hyshadew defaults sync` (or `--init-defaults`) before expecting bundled playlists under `/usr/share/hyshadew` to exist locally.

Other existing flags (e.g., `--fps`, `--antialias`) continue to work as global defaults when not overridden by playlist or per-item values.

## Concepts

- Target: Where a shader is displayed. Targets resolve dynamically per output:
  - If there is a visible workspace on an output (via Hyprland resolver), use `workspace:<name-or-id>`.
  - Otherwise, use `output:<name>`.
  - If neither has a matching mapping, the default playlist is used (if configured).
- Playlist: A named sequence of items with playback mode and defaults.
- Item: A single shader reference with optional overrides (duration, fps, antialias, refresh behavior).

## Config File Format (TOML)

Versioned format; stored anywhere, but recommended under `multi/`. The CLI accepts a file path. If a directory is passed, `default.toml` inside it is used.

```toml
# Required version for forwards-compatible parsing
version = 1

# Optional global crossfade used when switching visible workspaces on a monitor.
# If omitted, defaults to "1s". Set to 0 for hard cuts on workspace switches.
workspace_switch_crossfade = "1s"

[defaults]
# The default playlist name to use when a target has no explicit mapping.
playlist = "ambient"

# Optional global defaults (used if playlist/item do not specify):
# fps: number > 0 caps rendering; 0 or omitted = uncapped
# antialias: "auto" | "off" | integer sample count (2,4,8,16)
fps = 0
antialias = "auto"

[playlists.ambient]
# Playback mode: continuous (in order) or shuffle (permutation, reshuffled each cycle)
mode = "continuous"       # "continuous" | "shuffle"

# Default duration per item if not overridden. Either a number of seconds or humantime string.
item_duration = "300s"    # e.g., 90, "90s", "5m"

# Crossfade duration for item-to-item transitions within this playlist.
# If omitted, defaults to "1s". Set to 0 for a hard cut between items.
crossfade = "1s"

# Optional playlist-level defaults for fps/antialias
fps = 0                    # 0=uncapped
antialias = "auto"

[[playlists.ambient.items]]
handle = "local-shaders/grayday"  # directory or shadertoy handle
duration = "300s"
fps = 0
antialias = "auto"
# Refresh this item from remote once during this daemon session (ignored if --cache-only)
refresh_once = false

[[playlists.ambient.items]]
handle = "shadertoy://3dXyWj"
duration = "120s"
fps = 60
antialias = 4
refresh_once = true

[playlists.focus]
mode = "shuffle"
item_duration = "90s"
crossfade = "1s"
fps = 0
antialias = "auto"

[[playlists.focus.items]]
handle = "local-shaders/simplex"

[[playlists.focus.items]]
handle = "local-shaders/rotating-voronoise"

[targets]
# Map workspaces and outputs to playlists. Multiple selectors can point to the same playlist.
# Selectors:
#   workspace:<name-or-id>
#   output:<wl-output-name>
# Fallback:
#   _default = <playlist>

"output:DP-1" = "ambient"
"workspace:2" = "focus"
"_default" = "ambient"
```

### Field Reference

- `version` (required): integer, currently `1`.
- `workspace_switch_crossfade` (string|number, optional): duration to use when the visible workspace
  on a monitor changes, interrupting any current transition. Defaults to `"1s"`; `0` disables
  crossfading on workspace switches (hard cut).
- `[defaults]` (optional):
  - `playlist` (string): name of the default playlist for unmatched targets.
  - `fps` (number >= 0): default FPS cap. `0` or omitted = uncapped.
  - `antialias` (string|number): `"auto" | "off" | 2 | 4 | 8 | 16`.
- `[playlists.<name>]` (one or more):
  - `mode` (string): `"continuous" | "shuffle"`.
  - `item_duration` (string|number): default duration per item (defaults to `60s` when omitted).
  - `crossfade` (string|number, optional): duration for transitions between items in this playlist.
    Defaults to `"1s"`; `0` disables crossfade (hard cut).
  - `fps` (number >= 0, optional): playlist-level default FPS cap.
  - `antialias` (string|number, optional): playlist-level default AA.
  - `[[playlists.<name>.items]]` (one or more):
    - `handle` (string, required): shader handle. Examples:
      - `local-shaders/<dir>`
      - `shadertoy://<ID>`
      - absolute or relative pack path
      - `${MY_SHADER_PACK}` (environment variable expansion)
      - `~/shaders/demo` (home directory expansion)
      After expansion, relative paths search the current working directory, then the XDG config/data
      `local-shaders/` directories, and finally `/usr/share/hyshadew/local-shaders/`. Unset environment
      variables abort with a descriptive error.
    - `duration` (string|number, optional): per-item duration.
    - `fps` (number >= 0, optional): per-item FPS cap.
    - `antialias` (string|number, optional): per-item AA.
    - `refresh_once` (bool, optional, default `false`): if true, force a one-time remote refresh on the first activation of this item during the current daemon session (ignored when `--cache-only` is set).
- `[targets]` (optional):
  - Keys = selectors: `"workspace:<name-or-id>"`, `"output:<wl-output-name>"`, or `"_default"`.
  - Values = playlist names.

### Precedence (fps, antialias)

When resolving renderer settings, the following precedence applies (highest to lowest):

1) Item-level setting
2) Playlist-level setting
3) CLI global (`--fps`, `--antialias`)
4) Engine default (uncapped, `auto`)

### Validation Rules

- Unknown top-level keys → error.
- `playlists` must not be empty when `--multi` is used.
- Each playlist must contain at least one item.
- `handle` must be non-empty; strings that fail to parse into a shader handle produce an error at load or selection time.
- `mode` must be `continuous` or `shuffle`.
- `fps` must be a number `>= 0`; `0` means uncapped.
- `antialias` must be one of: `"auto"`, `"off"`, or integer sample count in `{2,4,8,16}`.
- `duration` and `item_duration` accept humantime (`"90s"`, `"5m"`) or a positive integer number of seconds.
- `crossfade` and `workspace_switch_crossfade` accept humantime or a non-negative integer seconds.
- `[targets]` may be omitted; if present, `"_default"` is recommended. In `--window` mode, `_default` is required.

## Scheduling Semantics

- Each target independently consumes its assigned playlist:
  - `continuous`: iterate items in order; loop back to start.
  - `shuffle`: generate a random permutation per cycle; reshuffle at end of cycle.
- Per-target runtime state: `current_index`, `started_at`, effective `duration`.
- On rollover (elapsed >= duration), advance to the next item and emit a selection change for that target.
- If an item fails to resolve or render, skip to the next item and log a warning with target and item handle.

### Transitions and Preemption

- Item-to-item transitions within a playlist use that playlist’s `crossfade` duration.
- Workspace-visible change on an output immediately interrupts any ongoing transition and starts a
  new crossfade using the global `workspace_switch_crossfade` duration. If set to `0`, the switch
  performs a hard cut.
- Starting a new transition cancels the previous blend; the old target becomes the new “from” frame
  only if it is still available; otherwise the compositor presents the new item directly.

## Target Resolution

- Each `wl_output` creates one wallpaper surface.
- For each output:
  1) If the active workspace is known and mapped: use `workspace:<name-or-id>`.
  2) Else if the output name is mapped: use `output:<name>`.
  3) Else use `_default` (when set).

Multiple outputs can share the same playlist; multiple workspaces can also share a playlist.

## Caching Semantics

- `--cache-only`: Global hard OFF for networking. Any per-item `refresh_once` is ignored. Items not present in cache fail with a clear error (and are skipped) rather than triggering a fetch.
- `--refresh`: Global one-time refresh per item per session (playlist mode). Implemented via a session-local “refreshed set” keyed by item handle/ID. On the first activation of an item, if `--refresh` is set or `refresh_once = true` on the item, refresh is attempted (unless `--cache-only` is set). Subsequent activations of the same item in the same daemon run do not re-fetch.
- Manual per-item control: `refresh_once = true` works even when global `--refresh` is not set.

## Window Mode Behavior

- `--window` with `--multi` is a playlist tester. Only the default playlist from `[defaults].playlist` is used; all `[targets]` mappings are ignored.
- If the default playlist is missing or misspelled, startup fails with a helpful error instructing the user to configure one.
- Transitions apply in window mode as specified: playlist `crossfade` between items; no workspace
  switch transitions since targets are ignored.

## Renderer Application

- For each target change, apply the item’s effective `fps` and `antialias` using the precedence rules.
- `surface_alpha` remains controlled by each pack’s `shader.toml` and is not overridden by playlists.
- No per-item size override is supported (intentionally excluded to avoid pathological configs).

### Crossfade Rendering Notes (non-normative)

- Implement crossfade by blending previous item output toward the next item output over the
  configured duration. Suggested approach: render next item normally while sampling the previous
  frame as a texture and applying a time-based mix.
- Crossfade duration `0` is a hard cut; very small durations should be clamped to avoid numerical
  instability.
- When a transition is preempted by a workspace switch, discard the in-progress blend and start a
  new transition using `workspace_switch_crossfade`.

## Module Boundaries (Implementation Sketch)

- New crate `multiconfig`:
  - Data types mirroring the TOML format.
  - Parsing + validation; returns a strongly-typed `MultiConfig`.
  - Utilities to compute effective settings for items (resolve precedence).

- New crate `scheduler`:
  - Playlist iteration logic per target (continuous/shuffle).
  - Session state and change emission (`tick(now) -> Vec<(TargetKey, SelectedItem)>`).
  - Refetch guard (session-local refreshed set) consulted for caching semantics.

- `hyshadew`:
  - CLI `--multi` and integration.
  - Target resolver (Wayland-only + Hyprland-aware resolver).
  - Orchestrates one `LayerSurface` per output and swaps shaders on selection changes.
  - Applies effective `fps`/`antialias` for each surface when items change.

- `renderer`:
  - Expose “swap shader” for an existing surface.
  - Keep current alpha behavior (`surface_alpha` from pack manifest applies via opaque region logic).

## Open Questions

- Shuffle reseed controls: add CLI/IPC to reseed or jump.
- Live reload: file-watch `--multi` for hot reload of playlists.
- Per-playlist overrides for future features (fill method, uniforms).

## Future Enhancements

- IPC control (skip/prev/pause/resume, jump to playlist/item, reseed shuffle).
- Hot-reload the multi config with safe handover of transitions.
- Day-parting/cron-like schedule constraints for items or playlists.
- Pre-fetch next item (and assets) to guarantee seamless transitions.
- Per-target overrides for fill method (stretch/center/tile) and uniform presets.
- Health checks and automatic fallback playlist on repeated failures.
- Telemetry hooks to export current item/playlist per target (for status bars).

## Examples

- Minimal single-playlist config with default only:

```toml
version = 1

[defaults]
playlist = "ambient"

[playlists.ambient]
mode = "continuous"
item_duration = "300s"

[[playlists.ambient.items]]
handle = "local-shaders/grayday"

[[playlists.ambient.items]]
handle = "shadertoy://3dXyWj"
refresh_once = true
```

- Per-workspace and per-output bindings:

```toml
version = 1

[defaults]
playlist = "ambient"
fps = 0
antialias = "auto"

[playlists.ambient]
mode = "continuous"
item_duration = "5m"

[[playlists.ambient.items]]
handle = "local-shaders/grayday"

[[playlists.ambient.items]]
handle = "local-shaders/fungal-apocalypse"
fps = 30
antialias = 4

[playlists.focus]
mode = "shuffle"
item_duration = "90s"

[[playlists.focus.items]]
handle = "local-shaders/simplex"

[[playlists.focus.items]]
handle = "local-shaders/rotating-voronoise"

[targets]
"workspace:2" = "focus"
"output:DP-1" = "ambient"
"_default" = "ambient"
```

## Implementation Plan

To make the rollout manageable, the work will proceed in the following stages. Each stage should compile, include tests, and be validated before moving on.

### Stage 1 – Configuration & Scheduling Foundations (complete)
1. Finalize the `multiconfig` crate:
   - Ensure the schema matches this spec, including validation and field defaults.
   - Add exhaustive unit tests for TOML parsing, validation failures, and precedence helpers.
2. Complete the `scheduler` crate:
   - Implement deterministic playlist iteration (continuous + shuffle) and per-target state tracking.
   - Support per-item duration overrides, FPS/AA carry-over, and crossfade metadata.
   - Provide unit tests covering rollover, shuffle reseeding, and multiple targets.

### Stage 2 – Renderer Surface Management & Crossfades (complete)
1. Wallpaper backend now keeps a `SurfaceState` per `wl_output` and exposes a `WallpaperRuntime` control channel for swapping shaders on existing surfaces without re-creating layer surfaces.
2. Shader swaps reuse `GpuState::set_shader`, blending the outgoing and incoming pipelines via additive color targets; zero-duration requests collapse to hard cuts.
3. Added `SurfaceId`/`OutputId` selectors, runtime surface snapshots, and unit tests validating fade weights and std140 layout so crossfades stay numerically stable.

### Stage 3 – Hyshadew Integration (Playlist Runtime) (complete)
1. Expand CLI/runtime:
   - Add `--multi` flag handling and enforce default playlist requirements in window mode.
   - Build a target resolver (Wayland-only first) and a simple Hyprland-aware variant when available.
2. Wire scheduler + renderer:
   - Instantiate scheduler, map targets, drive `tick` loop, and request shader swaps as selections change.
   - Implement refresh-once semantics using the scheduler’s metadata and existing repository caching flags.
   - Add integration tests (or smoke tests) that simulate playlist progression using the window mode.

### Stage 4 – Enhancements & Validation
1. Implement workspace-switch crossfade preemption and confirm transitions abort/resume correctly.
2. Add logging/telemetry to record playlist decisions and transitions for troubleshooting.
3. Document configuration usage (README/AGENTS.md) and provide sample configs under `multi/`.

Each stage builds on the previous one; stick to the sequence to minimize merge conflicts and keep the behavior testable throughout the implementation.

### Stage 5 - IPC, 

- Add file-watch
- Reload commands and other IPC mechanisms
- Add an "image" mode, which is a render pipeline which only shows an image. Then, enhance the multi spec and code to support playlists that can specify images. We should support single image, multipple

## TODO

- Revisit crossfade warm-up behaviour to eliminate residual frame jitter once additional profiling data is available.
