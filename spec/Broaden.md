# Broadening Linux Environment Support

This document outlines a staged plan to broaden Hyshadew beyond a Hyprland-first wallpaper workflow while keeping the renderer portable and fast. The focus is Linux; targets are Wayland (generic and vendor‑specific) and X11.

## Goals & Principles
- Be compositor‑agnostic by default; keep vendor specifics optional.
- Offer graceful degradation: animate when possible, fall back to high‑quality stills when not.
- Prefer existing desktop tools where appropriate (e.g., X11 static wallpapers) instead of re‑inventing.
- Keep a clean adapter abstraction so new backends are incremental work, not refactors.
- Provide clear diagnostics and user‑visible modes via CLI.

## Summary Checklist (for progress)

### Phase 1 — Refactors (staged)
- [ ] Stage 0: Type scaffolding (RenderPolicy, RuntimeOptions, TimeSource)
- [ ] Stage 1: Policy + time wiring (still mode hold, no export)
- [ ] Stage 2: FrameScheduler seam for cadence
- [ ] Stage 3: FillMethod + render_scale in renderer/wrapper
- [ ] Stage 4: Export pipeline (PNG first) via FrameSink
- [ ] Stage 5: AdapterProfile + adaptive heuristics (llvmpipe detection, occlusion caps)
- [ ] Stage 6: Telemetry + tests (scheduler reasons, PNG golden)

### Phase 1 — Features (after refactors)
- [ ] Still mode flags (`--still`, `--still-time`, `--still-random-seed`)
- [ ] Snapshot export (`--still-export <path.png>`)
- [ ] Adaptive FPS flags (`--fps-adaptive`, `--max-fps-occluded`)
- [ ] Quality controls (`--render-scale`, `--fill-method {stretch|center|tile}`)

### Phase 2 — Generic adapters & X11 stills
- [ ] Wayland layer‑shell adapter (BACKGROUND layer, per‑output)
- [ ] XDG output/scale + fractional scale handling
- [ ] Presentation‑time pacing hookup
- [ ] X11 stills documentation and helper commands (feh/xwallpaper)
- [ ] Window mode polish (borderless/bottom/sticky, `--output`)

### Phase 3 — X11 animator & Wayland multi‑output
- [ ] X11 root pixmap animator (x11rb/xcb, XRandR geometry)
- [ ] Retain pixmap + compositor restart handling
- [ ] Wayland multi‑output management + minimal damage
- [ ] Headless bake subcommands (best stills)

### Phase 4 — Vendor integrations / streaming (optional)
- [ ] GNOME: extension or still fallback via gsettings
- [ ] KDE Plasma: wallpaper plugin (QML) or fallback
- [ ] Streaming overlay wallpaper (where feasible)

## Phase 1 — Easy Tweaks (hours)

### Design Refactor for Phase 1 (before coding)
To implement still/exports/adaptive/quality controls cleanly and set the stage for later adapters, introduce a few small but high‑leverage abstractions. The goal is to separate “what to render” (policy), “how to render” (renderer + time), and “where to present” (presenter/sink), without reshaping the whole project.

- Runtime modes and policy
  - Add `RenderPolicy` capturing top‑level behaviour:
    - `Animate { target_fps: Option<f32>, adaptive: bool }`
    - `Still { time: Option<f32>, random_seed: Option<u64> }`
    - `Export { time: Option<f32>, path: PathBuf, format: Png|Exr }`
  - Add `RuntimeOptions` to consolidate Phase 1 flags: `render_scale`, `fill_method`, `max_fps_occluded`, `color_space`, `shader_compiler`, etc. This is passed into the renderer/runtime once at startup.
  - Introduce a `TimeSource` trait so the engine can switch between real time and fixed time easily:
    - `SystemTimeSource` (monotonic clock), `FixedTimeSource { t: f32 }` for still/exports.

- Presenter boundary (keep today’s backends; make the seam explicit)
  - Define a minimal `Presenter` trait (internally) that today is implemented by the existing `WindowRuntime` and `WallpaperRuntime` wrappers:
    - `prepare(&OutputInfo) -> SurfaceHandle`, `present(&SurfaceHandle, &Frame)`, `ack_frame()`.
    - For Phase 1, this is just an interface: the concrete implementations keep using winit/wayland glue as they do now.
  - Introduce a small `EngineLoop` that drives: `TimeSource` → render → `Presenter::present` → schedule next frame based on `RenderPolicy` (Animate vs Still/Export).

- Frame scheduling
  - Add a `FrameScheduler` that centralizes cadence decisions (target FPS, adaptive mode, occlusion handling). The scheduler determines whether to request another frame (`ControlFlow::Poll/Wait`) or to park after a still frame.

- Fill/coordinates and quality
  - Add `FillMethod` enum to `RendererConfig`: `Stretch | Center { content_size } | Tile { repeat }`. Implement mapping in the GLSL wrapper so Phase 1 can flip modes without compositor changes.
  - Wire `render_scale` into renderer: compute logical surface size = `requested_size * render_scale`; upscale in the wrapper.

- Snapshot/export pipeline
  - Introduce a `FrameSink` concept:
    - `SurfaceSink` (default wallpaper/window present) and `FileSink` (PNG/EXR write).
  - Add a helper to read back the final color attachment (sRGB aware) and encode via `image` or `png` crate; ensure color space agreed with `RendererConfig.color_space`.

- Capability and adaptive detection
  - Add `AdapterProfile` that records wgpu backend, adapter name/driver, and basic limits; use it to:
    - Emit a log hint when a software rasterizer is detected (llvmpipe/softpipe).
    - Default to `RenderPolicy::Still` or low FPS when software rendering is detected (unless overridden).

- Telemetry and errors
  - Keep path/handle logs from Phase 5; add per‑frame reason logs at debug level (e.g., "scheduler parked after still frame", "adaptive FPS reduced due to occlusion").

- Testing hooks
  - Unit: `TimeSource` switching, `FrameScheduler` cadence decisions, fill‑method coordinate transforms (small golden values).
  - Integration: export path writes a valid PNG with expected dimensions and sRGB mapping.

This refactor keeps external behaviour stable, simplifies Phase 1 features, and prepares for Phase 2/3 presenters without committing to their implementations yet.
- Still mode
  - Flags: `--still`, `--still-time <t|auto>`, `--still-random-seed`.
  - Behaviour: render exactly one frame, stop scheduling redraws; keep surface alive.
  - Optional: `--still-export <path.{png|exr}>` to snapshot without running wallpaper.
- Adaptive power
  - Detect software rasterizers (llvmpipe/softpipe) via wgpu adapter; auto‑hint or auto‑switch to still/low‑FPS.
  - `--fps-adaptive` lowers FPS when `frame` callbacks starve or window is occluded; `--max-fps-occluded <N>`.
- Quality and resolution controls
  - `--render-scale <0.25..1.0>` lower internal resolution; upscale in shader wrapper.
  - Fill modes: `--fill-method {stretch|center|tile}` implemented in the GLSL wrapper.

Deliverables
- CLI flags implemented; docs and help text updated.
- Snapshot pipeline in renderer; basic PNG export.
- Unit tests for flag parsing; manual acceptance on slow hardware.

## Phase 2 — Low Effort (days)
- Generic Wayland layer‑shell wallpaper
  - Use `zwlr_layer_shell_v1` at BACKGROUND layer; one surface per `wl_output`.
  - Handle `xdg-output` logical size/scale; use `wp_fractional_scale` if present; `presentation-time` for pacing.
  - Keep Hyprland integration optional on top (workspace signals).
- X11 snapshot integration (static background)
  - Pair `--still-export` with instructions (and optional helper) to set wallpaper via `feh`/`xwallpaper`.
- Window mode polish
  - `--borderless --always-on-bottom --sticky`; `--output <name>` to size/position a demo window per display.

Deliverables
- New adapter: Wayland layer‑shell (generic), gated behind capability detection.
- Quick‑start docs for X11 stills using existing tools.

## Phase 3 — Medium Effort (1–2 weeks)
- First‑class X11 wallpaper adapter
  - Use `x11rb`/`xcb` to set root pixmap (`_XROOTPMAP_ID`), update frames at target FPS; manage XRandR geometry.
  - Handle WM/compositor restarts, DPI/scale, retain pixmap to avoid flicker.
- Wayland output multiplexing
  - Create/resize one layer surface per output; handle hotplug/removal; map per‑output scale (HiDPI) and minimal damage.
- Headless bake
  - Batch render “best stills” of selected shaders into `~/.local/share/hyshadew/stills/` with presets.

Deliverables
- New adapter: X11 root pixmap animator.
- Robust multi‑output handling on Wayland.
- Headless export subcommands.

## Phase 4 — Harder Integrations (multi‑weeks)
- GNOME (Wayland)
  - No layer‑shell; options: GNOME Shell extension that hosts frames or fallback to still export + `gsettings`.
- KDE Plasma (Wayland)
  - Implement a Plasma wallpaper plugin (QML) that embeds frames or receives frames via IPC.
- Video/stream overlay wallpaper
  - “mpvpaper”‑style overlay (X11 `xwinwrap` analogue; Wayland requires compositor cooperation).

Deliverables
- Vendor‑specific plugins/extensions or documented fallbacks for stills.

## Slow Hardware Modes (can ship in Phase 1/2)
- High‑quality still presets
  - Per‑shader `still_at = <sec|auto>` in manifest; `--still-time auto` picks keyframes.
- Hybrid still + occasional refresh
  - `--still-refresh <interval>` recomputes one frame periodically; `--still-on-idle` animates only when idle (Wayland idle‑control).
- Progressive render
  - Render low‑res, upscale, refine over a few seconds to a final still.

## Abstractions & Architecture
- DisplayAdapter trait
  - Methods: enumerate outputs, (re)create/destroy surfaces, present frame, visibility hints, pacing hooks.
  - Adapters:
    - WaylandLayerShellAdapter (generic)
    - HyprlandAdapter (workspace cues over generic path)
    - X11RootPixmapAdapter
    - WindowAdapter (debug/demo)
- Capability detection
  - Probe Wayland protocols; detect compositor vendor; log clear fallbacks.
  - On X11, detect compositing, DPIs, screens.
- Diagnostics
  - `hyshadew doctor` prints adapters, protocols, wgpu backend/driver, FPS/occlusion, and recommended mode.

## CLI Surface (additive)
- Rendering: `--still`, `--still-time`, `--still-refresh`, `--still-on-idle`, `--render-scale`, `--fill-method`, `--fps-adaptive`, `--max-fps-occluded`.
- Export: `--export-png <path>`, `--export-exr <path>`, `--export-sequence <n> [--every <sec>]`.

## Testing & CI
- Unit: flag parsing, fill‑method transform, adapter feature probing.
- Integration: Wayland layer‑shell in nested compositor (sway nested), X11 root pixmap under Xephyr; snapshot comparisons.
- Performance: smoke tests for software rasterizer path (llvmpipe) to verify still/low‑FPS modes.

## Risks & Mitigations
- Compositor variability: lean on capability detection and fallbacks; keep vendor‑specific code optional.
- Power/perf regressions: make adaptive modes opt‑in with clear logs; document default behaviours.
- X11 quirks: test across common WMs; rely on established root‑pixmap conventions.

## Rollout Strategy
1) Phase 1: ship still/exports/adaptive/resolution controls; docs + defaults.
2) Phase 2: generic layer‑shell adapter; X11 still integration; polish window mode.
3) Phase 3: X11 animator + Wayland multi‑output; headless bake.
4) Phase 4: GNOME/KDE pathways or documented fallbacks; optional streaming overlays.
