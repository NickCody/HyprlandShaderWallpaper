# wax11 shader TODO

This document outlines future development tasks and specifications for wax11 shader, organized by major areas of focus.

## Broadening Linux Environment Support

This section outlines remaining work to broaden wax11 shader beyond Hyprland-first wallpaper workflow. The focus is Linux; targets are Wayland (generic and vendor‑specific) and X11.

### Phase 2 — Generic adapters & X11 stills
- Wayland layer‑shell adapter (BACKGROUND layer, per‑output)
- XDG output/scale + fractional scale handling
- Presentation‑time pacing hookup
- X11 stills documentation and helper commands (feh/xwallpaper)
- Window mode polish (borderless/bottom/sticky, `--output`)

### Phase 3 — X11 animator & Wayland multi‑output
- X11 root pixmap animator (x11rb/xcb, XRandR geometry)
- Retain pixmap + compositor restart handling
- Wayland multi‑output management + minimal damage
- Headless bake subcommands (best stills)

### Phase 4 — Vendor integrations / streaming (optional)
- GNOME: extension or still fallback via gsettings
- KDE Plasma: wallpaper plugin (QML) or fallback
- Streaming overlay wallpaper (where feasible)

### Slow Hardware Modes (can ship in Phase 2/3)
- High‑quality still presets
  - Per‑shader `still_at = <sec|auto>` in manifest; `--still-time auto` picks keyframes
- Hybrid still + occasional refresh
  - `--still-refresh <interval>` recomputes one frame periodically; `--still-on-idle` animates only when idle (Wayland idle‑control)
- Progressive render
  - Render low‑res, upscale, refine over a few seconds to a final still

### Architecture & Abstractions
- DisplayAdapter trait
  - Methods: enumerate outputs, (re)create/destroy surfaces, present frame, visibility hints, pacing hooks
  - Adapters:
    - WaylandLayerShellAdapter (generic)
    - HyprlandAdapter (workspace cues over generic path)
    - X11RootPixmapAdapter
    - WindowAdapter (debug/demo)
- Capability detection
  - Probe Wayland protocols; detect compositor vendor; log clear fallbacks
  - On X11, detect compositing, DPIs, screens
- Diagnostics
  - `wax11 doctor` prints adapters, protocols, wgpu backend/driver, FPS/occlusion, and recommended mode

### CLI Surface (additive)
- Rendering: `--still-refresh`, `--still-on-idle`
- Export: `--export-exr <path>`, `--export-sequence <n> [--every <sec>]`
- Window: `--borderless`, `--always-on-bottom`, `--sticky`, `--output <name>`

### Testing & CI
- Unit: adapter feature probing
- Integration: Wayland layer‑shell in nested compositor (sway nested), X11 root pixmap under Xephyr; snapshot comparisons
- Performance: smoke tests for software rasterizer path (llvmpipe) to verify still/low‑FPS modes

### Risks & Mitigations
- Compositor variability: lean on capability detection and fallbacks; keep vendor‑specific code optional
- Power/perf regressions: make adaptive modes opt‑in with clear logs; document default behaviours
- X11 quirks: test across common WMs; rely on established root‑pixmap conventions

## Public Release Preparation

### Release Automation
**Automate tagged releases with binaries**
Set up GitHub Actions to run formatting, clippy, and tests, then use `cargo dist` (or similar) on version tags to produce tar/zip bundles that include the binary, default manifests, and sample shaders. Upload artifacts with checksums and document curl/tar install steps. Expand to Homebrew taps or `.deb`/`.rpm` outputs when `cargo-dist` support is ready.

### Distribution Packaging
**Package for popular Linux distributions**
Start with an Arch AUR `-git` recipe that builds via `cargo build --release`, then graduate to a tagged package once releases stabilize. Provide a `flake.nix` offering `nix run .#wax11` and a reproducible dev shell. Explore Debian packaging (`cargo deb` or native `debian/` rules) and consider Flatpak when shader assets and GPU permissions are fully mapped out.

### Documentation and Policies
**Provide an optional installer helper**
Offer a POSIX `install.sh` (or `just install`) that copies the release binary into `~/.local/bin`, seeds configs under `~/.config/wax11/`, respects `--prefix`, and runs idempotently. Document cache/config paths so packagers can relocate assets to standard XDG directories.

**Formalize open-source policies and docs**
Include `LICENSE`, `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, and pointers from `AGENTS.md`. Sign release tags (GPG) for integrity. Ensure telemetry defaults to XDG-friendly cache/log locations and keep runbook-style notes so future maintainers understand diagnostics and tracing expectations.

### Future Configuration Extensions (Optional)
- Support profiles (`--profile NAME`) to namespace directories
- Add manifest schema migrations based on `defaults_version`
- Allow per-shader local override files layered above defaults
- Provide `wax11 doctor` for diagnosing missing directories or stale defaults

## Multi-Workspace Playlists

### IPC and Live Reload Features

#### File Watching and Hot Reload
- Live reload: file-watch `--playlist` for hot reload of playlists
- Hot-reload the multi config with safe handover of transitions

#### IPC Control System
- IPC control (skip/prev/pause/resume, jump to playlist/item, reseed shuffle)
- Shuffle reseed controls: add CLI/IPC to reseed or jump

#### Image Mode Support  
- Add an "image" mode render pipeline which only shows static images
- Enhance playlists to support image items alongside shader items
- Support single image and multiple image rotation within playlists

#### Advanced Scheduling Features
- Day-parting/cron-like schedule constraints for items or playlists
- Pre-fetch next item (and assets) to guarantee seamless transitions
- Health checks and automatic fallback playlist on repeated failures

#### Enhanced Customization
- Per-target overrides for fill method (stretch/center/tile) and uniform presets
- Per-playlist overrides for future features (fill method, uniforms)

#### Telemetry and Status
- Telemetry hooks to export current item/playlist per target (for status bars)

### Minor Optimizations
- Revisit crossfade warm-up behaviour to eliminate residual frame jitter once additional profiling data is available

## Crossfade Optimizations
- Introduce a lightweight `PrepareShader` worker so playlist swaps never block the Wayland render thread; keep the current thread focused on presenting frames.
- Reduce default warmup to roughly a single heartbeat (~100 ms) and only render the pending pipeline once preparation completes to avoid prolonged double-pass load.
- Emit preparation telemetry (start/end timestamps, fallback counters) so we can spot late arrivals and tune lead times.
- Future: layer in playlist lookahead plus a small pipeline cache once the async path proves stable.

## Path & Handle Specification

### Advanced Handle Schemes
- **`shader://` scheme**: Currently uses bare names instead of explicit `shader://pack` syntax
- **`playlist://` scheme**: Playlists are referenced by filename/path, not via `playlist://name` scheme
- **Flattened playlist layout**: Playlists remain in `playlists/` subdirectory rather than top-level `*.toml` files

### Enhanced Path Features
- **Search path configuration**: More granular control over search order and custom paths
- **Builtin shader schemes**: Reserved `builtin://` for hard-coded demo shaders
- **Advanced scheme extensibility**: Generic scheme parsing for future integrations (`steam://`, `gallery://`)

### Developer Experience
- **Better error messages**: Include search roots checked when resolution fails
- **Path diagnostics**: Enhanced debugging tools for path resolution issues

## Shader Compilation

### Documentation Enhancements
- Produce a dedicated user guide (`docs/`) that walks through manifest fields, playlist overrides, and troubleshooting examples
- Keep README as the quick reference once the dedicated guide exists

### Advanced Color Pipeline Features
- **Per-item overrides**: Consider extending multi-playlist TOML (and eventually manifests) with per-pass or per-playlist color overrides, honoring the CLI > playlist > manifest > default hierarchy
- **Shader wrapper helpers**: Add optional macros or uniforms so advanced shaders can explicitly request gamma↔linear conversions without retooling the pipeline

### Quality Assurance Tools
- **Validation tooling**: Capture side-by-side screenshots or numeric comparisons against Shadertoy for both gamma and linear modes to document expected output
