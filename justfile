set shell := ["bash", "-cu"]

# Install rustfmt/clippy components and pre-fetch dependencies
setup:
	rustup component add rustfmt clippy
	cargo fetch

# Format check and lint the entire workspace
check:
	cargo fmt --all --check
	cargo clippy --all-targets --all-features -- -D warnings

# Run the default windowed demo shader
run-demo:
	cargo run -p wallshader -- --window --shadertoy https://www.shadertoy.com/view/3dXyWj

# Run the playlist sample configuration
run-playlist:
	cargo run -p wallshader -- --playlist workspaces.toml

# Mirror the GitHub Actions workflow (build + tests + clippy)
validate:
	cargo build --verbose
	cargo test --verbose
	cargo clippy --all-targets --all-features -- -D warnings
