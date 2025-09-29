- [ ] Baseline repository setup for contributors
- [ ] Publish `hyshadew` via Cargo install
- [ ] Automate tagged releases with binaries
- [ ] Package for popular Linux distributions
- [ ] Provide an optional installer helper
- [ ] Formalize open-source policies and docs

## Baseline repository setup for contributors
Pin the Rust toolchain (`rust-toolchain.toml`), list system dependencies (Wayland dev packages, GPU drivers) in `README.md`, and document the quick start commands (`git clone`, `cargo build --release`, `cargo run -p hyshadew -- --help`). Add a `justfile`/`Makefile` with `setup`, `check`, and demo targets so newcomers have reproducible entry points. Tag releases and maintain a `CHANGELOG.md` using Keep a Changelog plus semantic versioning for downstream consumers.

## Publish `hyshadew` via Cargo install
Clean up the binary crate for crates.io, ensure it builds from a fresh checkout, and release tagged versions so users can `cargo install hyshadew`. Until crates.io publication, support `cargo install --git ... --tag vX.Y.Z hyshadew` and verify shader assets or configs are bundled appropriately for install-time use.

## Automate tagged releases with binaries
Set up GitHub Actions to run formatting, clippy, and tests, then use `cargo dist` (or similar) on version tags to produce tar/zip bundles that include the binary, default manifests, and sample shaders. Upload artifacts with checksums and document curl/tar install steps. Expand to Homebrew taps or `.deb`/`.rpm` outputs when `cargo-dist` support is ready.

## Package for popular Linux distributions
Start with an Arch AUR `-git` recipe that builds via `cargo build --release`, then graduate to a tagged package once releases stabilize. Provide a `flake.nix` offering `nix run .#hyshadew` and a reproducible dev shell. Explore Debian packaging (`cargo deb` or native `debian/` rules) and consider Flatpak when shader assets and GPU permissions are fully mapped out.

## Provide an optional installer helper
Offer a POSIX `install.sh` (or `just install`) that copies the release binary into `~/.local/bin`, seeds configs under `~/.config/hyshadew/`, respects `--prefix`, and runs idempotently. Document cache/config paths so packagers can relocate assets to standard XDG directories.

## Formalize open-source policies and docs
Include `LICENSE`, `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, and pointers from `AGENTS.md`. Sign release tags (GPG) for integrity. Ensure telemetry defaults to XDG-friendly cache/log locations and keep runbook-style notes so future maintainers understand diagnostics and tracing expectations.
