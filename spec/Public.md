# Public Release Spec

## Remaining Tasks

### Release Automation
**Automate tagged releases with binaries**
Set up GitHub Actions to run formatting, clippy, and tests, then use `cargo dist` (or similar) on version tags to produce tar/zip bundles that include the binary, default manifests, and sample shaders. Upload artifacts with checksums and document curl/tar install steps. Expand to Homebrew taps or `.deb`/`.rpm` outputs when `cargo-dist` support is ready.

### Distribution Packaging
**Package for popular Linux distributions**
Start with an Arch AUR `-git` recipe that builds via `cargo build --release`, then graduate to a tagged package once releases stabilize. Provide a `flake.nix` offering `nix run .#wallshader` and a reproducible dev shell. Explore Debian packaging (`cargo deb` or native `debian/` rules) and consider Flatpak when shader assets and GPU permissions are fully mapped out.

### Documentation and Policies
**Provide an optional installer helper**
Offer a POSIX `install.sh` (or `just install`) that copies the release binary into `~/.local/bin`, seeds configs under `~/.config/wallshader/`, respects `--prefix`, and runs idempotently. Document cache/config paths so packagers can relocate assets to standard XDG directories.

**Formalize open-source policies and docs**
Include `LICENSE`, `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, and pointers from `AGENTS.md`. Sign release tags (GPG) for integrity. Ensure telemetry defaults to XDG-friendly cache/log locations and keep runbook-style notes so future maintainers understand diagnostics and tracing expectations.

### Future Configuration Extensions (Optional)
- Support profiles (`--profile NAME`) to namespace directories
- Add manifest schema migrations based on `defaults_version`
- Allow per-shader local override files layered above defaults
- Provide `wallshader doctor` for diagnosing missing directories or stale defaults

## Go-Live Checklist
- Confirm automation for tagged releases is green and artifacts include defaults
- Validate distribution packages or recipes exist for at least Arch/Nix  
- Ship the installer helper and document idempotent usage
- Publish open-source policy docs and link them in surfaced materials
- Run a final smoke test covering cargo install, the installer workflow, and user override workflows to ensure the public release path is smooth