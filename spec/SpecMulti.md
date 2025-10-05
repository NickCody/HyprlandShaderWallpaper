# Multi-Workspace Playlists (Spec)

## Remaining Tasks

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