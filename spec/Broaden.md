# Broadening Linux Environment Support

This document outlines remaining work to broaden WallShader beyond Hyprland-first wallpaper workflow. The focus is Linux; targets are Wayland (generic and vendor‑specific) and X11.

## Remaining Work

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

## Slow Hardware Modes (can ship in Phase 2/3)
- High‑quality still presets
  - Per‑shader `still_at = <sec|auto>` in manifest; `--still-time auto` picks keyframes
- Hybrid still + occasional refresh
  - `--still-refresh <interval>` recomputes one frame periodically; `--still-on-idle` animates only when idle (Wayland idle‑control)
- Progressive render
  - Render low‑res, upscale, refine over a few seconds to a final still

## Architecture & Abstractions
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
  - `wallshader doctor` prints adapters, protocols, wgpu backend/driver, FPS/occlusion, and recommended mode

## CLI Surface (additive)
- Rendering: `--still-refresh`, `--still-on-idle`
- Export: `--export-exr <path>`, `--export-sequence <n> [--every <sec>]`
- Window: `--borderless`, `--always-on-bottom`, `--sticky`, `--output <name>`

## Testing & CI
- Unit: adapter feature probing
- Integration: Wayland layer‑shell in nested compositor (sway nested), X11 root pixmap under Xephyr; snapshot comparisons
- Performance: smoke tests for software rasterizer path (llvmpipe) to verify still/low‑FPS modes

## Risks & Mitigations
- Compositor variability: lean on capability detection and fallbacks; keep vendor‑specific code optional
- Power/perf regressions: make adaptive modes opt‑in with clear logs; document default behaviours
- X11 quirks: test across common WMs; rely on established root‑pixmap conventions