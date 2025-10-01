- [x] Baseline repository setup for contributors
- [x] Publish `hyshadew` via Cargo install
- [ ] Automate tagged releases with binaries
- [ ] Package for popular Linux distributions
- [ ] Provide an optional installer helper
- [ ] Formalize open-source policies and docs
- [ ] Implement defaults & user configuration

## Baseline repository setup for contributors
Pin the Rust toolchain (`rust-toolchain.toml`), list system dependencies (Wayland dev packages, GPU drivers) in `README.md`, and document the quick start commands (`git clone`, `cargo build --release`, `cargo run -p hyshadew -- --help`). Add a `justfile`/`Makefile` with `setup`, `check`, and demo targets so newcomers have reproducible entry points. Tag releases and maintain a `CHANGELOG.md` using Keep a Changelog plus semantic versioning for downstream consumers.

## Publish `hyshadew` via Cargo install
Document the `cargo install --git https://github.com/NickCody/HyprlandShaderWallpaper --tag v0.9.1 --locked hyshadew` path (or `--branch main` while iterating) and verify it builds from a clean checkout. When ready for crates.io, polish metadata and publish tagged versions so users can install without the `--git` flag.

## Automate tagged releases with binaries
Set up GitHub Actions to run formatting, clippy, and tests, then use `cargo dist` (or similar) on version tags to produce tar/zip bundles that include the binary, default manifests, and sample shaders. Upload artifacts with checksums and document curl/tar install steps. Expand to Homebrew taps or `.deb`/`.rpm` outputs when `cargo-dist` support is ready.

## Package for popular Linux distributions
Start with an Arch AUR `-git` recipe that builds via `cargo build --release`, then graduate to a tagged package once releases stabilize. Provide a `flake.nix` offering `nix run .#hyshadew` and a reproducible dev shell. Explore Debian packaging (`cargo deb` or native `debian/` rules) and consider Flatpak when shader assets and GPU permissions are fully mapped out.

## Provide an optional installer helper
Offer a POSIX `install.sh` (or `just install`) that copies the release binary into `~/.local/bin`, seeds configs under `~/.config/hyshadew/`, respects `--prefix`, and runs idempotently. Document cache/config paths so packagers can relocate assets to standard XDG directories.

## Formalize open-source policies and docs
Include `LICENSE`, `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, and pointers from `AGENTS.md`. Sign release tags (GPG) for integrity. Ensure telemetry defaults to XDG-friendly cache/log locations and keep runbook-style notes so future maintainers understand diagnostics and tracing expectations.

## Implement defaults & user configuration
Deliver a robust, user-first configuration model by executing the following phases:

### Phase 1 — Path Infrastructure *(complete)*
1. Introduce a `paths` module (e.g. `crates/hyshadew/src/paths.rs`) that wraps `directories_next::ProjectDirs` and exposes:
   - `config_dir` (`$XDG_CONFIG_HOME/hyshadew`, default `~/.config/hyshadew`).
   - `data_dir` (`$XDG_DATA_HOME/hyshadew`, default `~/.local/share/hyshadew`).
   - `cache_dir` (`$XDG_CACHE_HOME/hyshadew`, default `~/.cache/hyshadew`).
   - `system_share_dir` (`/usr/share/hyshadew` on Unix, overridable via env/build settings).
2. Provide helpers returning ordered search roots for shaders, playlists, and caches, differentiating between writable and read-only roots.
3. Support env-var overrides (`HYSHADEW_CONFIG_DIR`, `HYSHADEW_DATA_DIR`, `HYSHADEW_CACHE_DIR`, `HYSHADEW_SHARE_DIR`) with documented precedence (CLI flag > env > XDG default).
4. Update config and asset loaders to rely on the module instead of ad-hoc path handling.

Implemented in commit-in-progress: `AppPaths` now resolves XDG directories (config, data, cache) plus `/usr/share/hyshadew`, supports `HYSHADEW_*` overrides, and the wallpaper/multi entry points use the computed shader/cache roots.

### Phase 2 — Directory Bootstrapping *(complete)*
1. Ensure required directories exist during startup and create them with clear error handling.
2. Seed logical subfolders (`config/local-shaders`, `config/multi`, `data/local-shaders`, `data/multi`, `cache/shadertoy`).
3. Write a `state.toml` in `config_dir` capturing metadata (defaults version, last sync, flags).
4. Emit telemetry when directories are created or missing for easier diagnostics.

Implemented via `bootstrap::bootstrap_filesystem`: it creates the XDG directory tree, initialises shader/playlist subfolders plus the Shadertoy cache, and materialises `state.toml` with default metadata while logging each action.

### Phase 3 — Default Content Packaging & Sync *(complete)*
1. Treat `/usr/share/hyshadew` as the authoritative source for bundled shader packs, manifests, and docs.
2. Copy only missing defaults into user directories during first run or when explicitly requested.
3. Track a monotonically increasing `defaults_version` (e.g. file in `/usr/share/hyshadew/VERSION`). Compare against `state.toml` and prompt for sync when upstream content is newer.
4. Provide a dry-run option that reports pending copies without making changes.

Implemented via `defaults::sync_defaults`: it scans the configured share directory, copies any missing shader packs or playlists into `~/.local/share/hyshadew`, respects a `VERSION` stamp for logging, records the last sync timestamp, and updates `state.toml` only when content is actually installed.

### Phase 4 — CLI Surface & Commands
1. Add `hyshadew defaults` subcommands:
   - `sync` to copy missing/newer defaults, with `--force` that overwrites after backing up.
   - `list` to enumerate system defaults and their user-space status.
   - `where` to print resolved paths for debugging.
2. Respect env overrides and CLI flags when executing commands.
3. Support a daemon flag (`--init-defaults`) for one-shot setup.
4. Log which search root satisfies each shader to aid support.

### Phase 5 — Path Resolution Semantics
1. Define resolution rules used across the app:
   - Absolute paths are honored verbatim.
   - Relative paths search the process working directory, user roots, then `/usr/share/hyshadew`.
2. Implement interpolation for config values:
   - Expand `${VAR}` placeholders using `std::env::var`; emit a descriptive error if unset.
   - Expand `~` to the user home directory.
   - Run interpolation before resolution and reuse a shared error type for clarity.
3. Record resolved paths in telemetry for troubleshooting.
4. Add unit tests covering each rule and failure case.

### Phase 6 — Documentation & Telemetry
1. Update `README.md`, `AGENTS.md`, and `spec/SpecMulti.md` with the directory layout, override mechanics, and CLI usage.
2. Document workflows for copying defaults (`cp -R /usr/share/hyshadew/... ~/.local/share/hyshadew/...`).
3. Clarify env-var interpolation semantics and failure modes in user docs.
4. Instrument logs (`info`/`warn`/`debug`) to capture sync actions, missing assets, and resolution traces.

### Phase 7 — Testing & CI Integration
1. Add integration tests using temporary directories to simulate fresh installs, upgrades, and env overrides.
2. Include a CI smoke test that exercises `hyshadew defaults sync` into a temp tree and verifies expected files.
3. Guarantee idempotency when rerunning sync and ensure user modifications persist.
4. Validate cross-platform share-path handling (macOS/BSD) via conditional compilation and targeted tests.

### Phase 8 — Future Extensions (Optional)
- Support profiles (`--profile NAME`) to namespace directories.
- Add manifest schema migrations based on `defaults_version`.
- Allow per-shader local override files layered above defaults.
- Provide `hyshadew doctor` for diagnosing missing directories or stale defaults.

### Implementation Notes
- Use atomic file operations when copying defaults; write to temp files then rename.
- Preserve file permissions (consider `fs_extra` or manual metadata handling).
- When forcing overwrites, create timestamped backups (`.bak`).
- Leverage structured errors (`thiserror`/`anyhow`) so CLI output is actionable.

### Acceptance Criteria
- Fresh installs create user directories automatically and surface defaults without manual steps.
- TOML configs support `${VAR}` and `~` expansion; missing variables fail with clear messages.
- Reinstalls/upgrades preserve user-modified files while exposing new defaults via explicit sync.
- Documentation and telemetry clearly communicate asset locations and customization workflows.

## Go-Live Checklist
- Confirm automation for tagged releases is green and artifacts include defaults (`Automate tagged releases`).
- Validate distribution packages or recipes exist for at least Arch/Nix (`Package for popular Linux distributions`).
- Ship the installer helper and document idempotent usage (`Provide an optional installer helper`).
- Publish open-source policy docs and link them in surfaced materials (`Formalize open-source policies`).
- Complete Phases 1–7 of the defaults configuration plan; decide on optional Phase 8 scope.
- Run a final smoke test covering cargo install, defaults sync, and user override workflows to ensure the public release path is smooth.
