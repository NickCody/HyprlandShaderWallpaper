# Rebranding Summary: WAX11 → wax11 shader

## Overview

The project has been successfully rebranded from **WAX11** to **wax11 shader** (full name) with **wax11** as the binary name.

**Key naming conventions:**
- **Full project name:** "wax11 shader" (lowercase, Wayland/X11 shader)
- **Binary name:** `wax11`
- **Environment variables:** `WAX11_*`
- **Directories:** `.../wax11/`
- **Repository:** `github.com/NickCody/wax11`
- **AppImage:** `wax11-x86_64.AppImage`

## Changes Made

### 1. Build System & Crates
- ✅ Renamed `crates/wax11` → `crates/wax11`
- ✅ Updated `Cargo.toml` workspace metadata:
  - Repository: `https://github.com/NickCody/wax11`
  - Keywords: kept `hyprland`, `wayland`, added `x11`, `shadertoy`, `wallpaper`
  - Package name: `wax11`
  - Binary name: `wax11`
- ✅ Updated all crate cross-references

### 2. Environment Variables
- `WAX11_CONFIG_DIR` → `WAX11_CONFIG_DIR`
- `WAX11_DATA_DIR` → `WAX11_DATA_DIR`
- `WAX11_CACHE_DIR` → `WAX11_CACHE_DIR`
- `WAX11_SHARE_DIR` → `WAX11_SHARE_DIR`
- `WAX11_DEV_ROOT` → `WAX11_DEV_ROOT`

**Legacy migration:** Old `WAX11_*` variables are now treated as legacy fallbacks in `paths.rs`, allowing smooth migration for existing users.

### 3. Directory Paths
- `~/.config/wax11` → `~/.config/wax11`
- `~/.local/share/wax11` → `~/.local/share/wax11`
- `~/.cache/wax11` → `~/.cache/wax11`
- `/usr/share/wax11` → `/usr/share/wax11`
- `opt/wax11` → `opt/wax11`

**Legacy migration:** Old directories are automatically migrated on first run via `paths.rs`.

### 4. Documentation
- ✅ `README.md` - Updated all references to wax11 shader, binary commands, URLs, paths
- ✅ `AGENTS.md` - Updated project description, environment variables, paths, debugging paths
- ✅ `CHANGELOG.md` - Updated project name
- ✅ `CONTRIBUTING.md` - Updated project references
- ✅ `TODO.md` - Updated project name
- ✅ `LICENSE` - Updated copyright holder to "wax11 shader Contributors"
- ✅ `.github/README.md` - Updated project references
- ✅ `.github/BRANCH_PROTECTION.md` - Updated repository URLs

### 5. Scripts
- ✅ `scripts/install.sh` - All references updated (paths, binary name, repository URL)
- ✅ `scripts/launch-local` - Updated binary and environment variables
- ✅ `scripts/launch-local-async` - Updated binary and environment variables
- ✅ `justfile` - Updated package name (`-p wax11`) and GitHub URLs

### 6. AppImage & Packaging
- ✅ `AppImageBuilder.yml`:
  - App ID: `io.github.nickcody.wax11`
  - App name: `wax11 shader`
  - Binary: `usr/bin/wax11`
  - Icon: `wax11`
  - AppImage filename: `wax11-x86_64.AppImage`
  - All environment variables and paths updated
- ✅ Desktop files renamed and updated:
  - `wax11.desktop` → `wax11.desktop`
  - `io.github.nickcody.wax11.desktop` → `io.github.nickcody.wax11.desktop`
- ✅ Icon renamed: `wax11.svg` → `wax11.svg` (icon text updated from "WS" to "W")

### 7. GitHub Workflows
- ✅ `.github/workflows/ci.yml` - Updated AppImage names, artifacts, binary paths
- ✅ `.github/workflows/cache-optimization.yml` - Updated references

### 8. Code Updates
- ✅ All Rust source files updated:
  - Thread names: `"wax11-*"` → `"wax11-*"`
  - Labels: `"wax11 *"` → `"wax11 *"`
  - Debug paths: `/tmp/wax11_wrapped.frag` → `/tmp/wax11_wrapped.frag`
  - GLSL macro prefixes: `wax11_*` → `wax11_*` (e.g., `wax11_Fill` → `wax11_Fill`)
  - Comments and documentation updated throughout

### 9. Internal GLSL Shaders
Updated all internal shader macros and uniforms:
- `wax11_mix` → `wax11_mix`
- `wax11_channel*_texture` → `wax11_channel*_texture`
- `wax11_channel*_sampler` → `wax11_channel*_sampler`
- `wax11_Surface` → `wax11_Surface`
- `wax11_Fill` → `wax11_Fill`
- `wax11_FillWrap` → `wax11_FillWrap`
- `wax11_gl_FragCoord` → `wax11_gl_FragCoord`

## Migration Path for Users

### Automatic Migration
The code includes automatic migration logic in `crates/wax11/src/paths.rs`:
1. Old `WAX11_*` environment variables are treated as legacy fallbacks
2. Old directories (`~/.config/wax11`, etc.) are automatically renamed to new paths
3. Legacy organization paths (`WAX11ade`) are migrated to new structure

### Manual Steps for Users
Users upgrading from WAX11 to wax11 shader can:
1. Rename environment variables in their shell configs (optional, but recommended)
2. Update any custom scripts that call `wax11` → `wax11`
3. Update desktop launchers and systemd services to use `wax11` binary

### Breaking Changes
- Binary name changed: `wax11` → `wax11`
- All paths changed: `*/wax11/` → `*/wax11/`
- Environment variables changed: `WAX11_*` → `WAX11_*`
- Repository URL changed: `.../WAX11` → `.../wax11`

## Validation

Build check passed:
```bash
cargo check
# Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.19s
```

All grep searches for old references returned only intentional legacy migration constants.

## Next Steps

1. Test the build system: `cargo build --release`
2. Test AppImage generation: `appimage-builder --recipe AppImageBuilder.yml`
3. Test the installer script: `bash scripts/install.sh --help`
4. Update CI/CD badges if needed
5. Create a new release with the rebranded name
6. Update any external documentation or links

## Notes

- Keywords retained `hyprland` and `wayland` as they remain relevant technical terms
- Legacy migration code ensures smooth transition for existing users
- All thread names, debug labels, and internal identifiers updated for consistency
- GLSL wrapper macros updated to maintain branding consistency even in generated code
